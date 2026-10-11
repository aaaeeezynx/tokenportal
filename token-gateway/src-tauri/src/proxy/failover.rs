//! 來源／協議的自動換手。
//!
//! 為什麼需要這一層：**同一個來源的模型可能逐模型只在一種端點上架**。2026-09-30
//! 對 opencode-go 直接量測：`grok-4.7`／`gpt-6-luna`／`muse-spark-1.3-contributor`
//! 只在 `/responses` 有回應，`mimo-v2.6-pro`／`flash` 只在 `/chat/completions`，
//! `deepseek-*` 兩邊都有。所以「來源協議選對了」不等於「這個模型打得通」：
//! 上游會回 `400 ModelProtocolUnsupported`。
//!
//! 這裡把「候選（來源, 協議）」排成一條佇列（抄 cc-switch 的 failover queue 精神），
//! 上游回協議不支援、連線失敗或 5xx 就換下一個；**並且補上 cc-switch 沒做的那一格**：
//! 同一個來源也換協議重試（Responses 入站時 chat ↔ responses 都生得出請求體）。
//! 成功後把「這個來源的這個模型該用哪個協議」**落庫**（`provider_model_protocol`），
//! 下次第一個就試它 —— 落庫的理由與 `provider_stripped_fields` 當年一樣：
//! 只放記憶體的話，網關每次重啟都要為每個這種模型重踩一次 400。
//!
//! 已知限制（照實寫在文件裡）：Anthropic 入站的請求**只生得出 chat 請求體**
//! （`Anthropic → Responses` 沒有翻譯器，矩陣直接回 `E_ANTHROPIC_UNSUPPORTED`），
//! 所以 responses-only 的模型在 Claude Code 永遠換不過去。

use super::*;

/// 一次上游嘗試：用哪個來源、哪個協議。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Attempt {
    pub(super) provider_id: i64,
    pub(super) api_format: String,
}

impl Attempt {
    pub(super) fn new(provider_id: i64, api_format: &str) -> Self {
        Self {
            provider_id,
            api_format: api_format.to_string(),
        }
    }
}

/// 學到的協議：落庫在 `provider_model_protocol`（見 `crate::trace::protocol`）。
///
/// **為什麼不只放記憶體**：網關每次重啟就會忘掉，於是每個「宣告協議與實際不符」的
/// 模型都要多付一次失敗探測（一次 400）。這與 `provider_stripped_fields` 當年的
/// 問題一模一樣，所以用同一種做法（短命連線、錯誤一律吞掉）。
///
/// 兩個維度是必要的：
/// - **資料庫**：由 `db_path` 決定連到哪個庫，`provider_id` 只在單一資料庫內唯一。
/// - **宣告協議**：使用者把來源協議從 `openai-chat` 改成 `openai-responses` 是
///   **明確的設定變更**，不可以被舊的學習結果蓋掉（那會讓「改了設定卻沒生效」）——
///   它是表的主鍵之一。
///
/// 記住「這個來源的這個模型，在宣告協議 X 之下用 Y 會通」。
pub(super) fn learn_format(
    db_path: &std::path::Path,
    provider_id: i64,
    model: &str,
    declared_fmt: &str,
    used_fmt: &str,
) {
    let Ok(conn) = open_conn(db_path) else {
        return;
    };
    let _ = trace::remember_protocol(&conn, provider_id, model, declared_fmt, used_fmt);
}

/// 上次學到的協議（沒有就 `None`）。**只在宣告協議相同時才回傳。**
pub(super) fn learned_format(
    db_path: &std::path::Path,
    provider_id: i64,
    model: &str,
    declared_fmt: &str,
) -> Option<String> {
    let conn = open_conn(db_path).ok()?;
    trace::load_protocol(&conn, provider_id, model, declared_fmt)
}

/// 這個協議「另一邊」有哪些可換的具體協議。
///
/// `mixed` 不是一種真的線上協議（它的意思是「兩種端點都直通」），所以它的另一邊
/// 同時包含 chat 與 responses：遇到 chat-only 的模型時要換成 `openai-chat`
/// （翻譯那條），遇到 responses-only 時直通本來就會過。
fn other_sides(fmt: &str) -> &'static [&'static str] {
    match fmt {
        "openai-chat" => &["openai-responses"],
        "openai-responses" => &["openai-chat"],
        "mixed" => &["openai-chat", "openai-responses"],
        // 理論上 `formats_for` 會先展開 auto，這裡不會被走到；留著是防禦
        "auto" => &["openai-chat", "openai-responses", "anthropic"],
        // 明確宣告 anthropic 的來源：另一邊是 chat（Responses→Anthropic 沒翻譯器）
        "anthropic" => &["openai-chat"],
        _ => &[],
    }
}

/// 換協議是否有意義：矩陣必須支援，**而且**翻譯種類真的不同。
///
/// - Responses 入站：chat→responses＝直通 vs 翻譯，明顯不同 ✅
/// - Anthropic 入站：chat→responses 矩陣直接 Err（沒有 Anthropic→Responses 翻譯器）❌
/// - OpenAI chat 入站：chat→responses 也是 Err（B6 的明確報錯）❌
fn flip_is_useful(in_fmt: InFmt, from: &str, to: &str) -> bool {
    match (
        resolve_trans_kind(in_fmt, TargetFmt::from_db(from)),
        resolve_trans_kind(in_fmt, TargetFmt::from_db(to)),
    ) {
        (Ok(a), Ok(b)) => a != b,
        _ => false,
    }
}

/// 一個來源在這個模型上的協議候選（保序、去重）。
///
/// **有效協議 = 逐模型實測優先，其次來源宣告。**
///
/// 為什麼要有逐模型這一層：協議是「某個模型在某個上游」的屬性，不是「來源」的
/// 屬性。訂閱制的 opencode-go 單一 base_url、單一金鑰底下，`claude-haiku-5-5`
/// 只吃 Anthropic Messages、其餘模型吃 OpenAI chat。用來源級欄位描述不了，
/// 使用者只能把同一個訂閱拆成兩個來源 —— 金鑰重複，App 的用量／成本／健康度
/// 統計也被切碎（而上游那邊本來就只有一份額度）。
///
/// `auto`（或兩者都空）代表**不知道**：那就依入站格式先試最自然的那一種
/// ——客戶端自己用的協議，通常就是這個模型支援的協議 ——再試其餘兩種。
/// 這個順序讓「一個來源服務多種客戶端」在第一次請求就大概率命中，
/// 猜錯也只是一次失敗往返，之後由 `learn_format` 記住。
fn formats_for(in_fmt: InFmt, declared: &str, model_protocol: Option<&str>) -> Vec<String> {
    let explicit = model_protocol
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .unwrap_or_else(|| declared.trim());
    if explicit.is_empty() || explicit.eq_ignore_ascii_case("auto") {
        let primary = match in_fmt {
            InFmt::Anthropic => "anthropic",
            InFmt::Responses => "openai-responses",
            InFmt::OpenAi => "openai-chat",
            // 中性路徑（/v1/models 之類）：不猜，照 chat 先試
            _ => "openai-chat",
        };
        let mut v = vec![primary.to_string()];
        for p in ["openai-chat", "openai-responses", "anthropic"] {
            if p != primary {
                v.push(p.to_string());
            }
        }
        return v;
    }
    let mut v = vec![explicit.to_string()];
    for f in other_sides(explicit) {
        v.push(f.to_string());
    }
    v
}

/// 排候選佇列（去重、保序）。
///
/// 順序：① 上次學到的協議 ② 綁定來源的逐模型協議／宣告協議 ③ 該來源的其餘協議
/// ④ 其他也登記了這個模型的來源（各自同一套順序，依 priority/id 排序）。
pub(super) fn plan_attempts(
    bound_pid: i64,
    bound_fmt: &str,
    bound_model_protocol: Option<&str>,
    in_fmt: InFmt,
    learned: Option<&str>,
    others: &[ReroutedProvider],
) -> Vec<Attempt> {
    let mut out: Vec<Attempt> = Vec::new();
    let add = |out: &mut Vec<Attempt>, pid: i64, fmt: &str| {
        let fmt = fmt.trim();
        if fmt.is_empty() {
            return;
        }
        if out
            .iter()
            .any(|a| a.provider_id == pid && a.api_format.eq_ignore_ascii_case(fmt))
        {
            return;
        }
        out.push(Attempt::new(pid, fmt));
    };
    if let Some(l) = learned {
        add(&mut out, bound_pid, l);
    }
    // 第一個候選**無條件**排入：矩陣若不支援這一格（例如 Gemini 入站、或
    // Responses 入站碰上 anthropic），要讓它帶著那格專屬的明確訊息回報，
    // 而不是被靜默濾掉、最後只回一句籠統的「所有協議都不被接受」。
    //
    // 其餘候選的比較基準是**第一個候選**，不是 `bound_fmt` —— 有逐模型協議時
    // 兩者不同。拿來源宣告去比會誤判：宣告 `mixed` + 模型協議 `anthropic` 時，
    // `(Anthropic, Mixed)` 與 `(Anthropic, OpenAiChat)` 都是 `AnthropicToChat`
    // （同一種翻譯），於是「換到 chat」被當成沒用 —— 但實際上第一個候選是
    // `anthropic`（直通 `/messages`），換到 chat 是**完全不同的路徑**。
    //
    // `auto` 沒有「原本的協議」可比，不能套 `flip_is_useful` —— 它的第一邊會是
    // Auto → Err → 恆為 false，候選會全被濾掉、排出空清單。改成直接問矩陣。
    let auto_mode = bound_fmt.trim().is_empty() || bound_fmt.eq_ignore_ascii_case("auto");
    let cands = formats_for(in_fmt, bound_fmt, bound_model_protocol);
    let from = cands.first().cloned().unwrap_or_default();
    for (idx, f) in cands.iter().enumerate() {
        let keep = if idx == 0 {
            true
        } else if auto_mode {
            resolve_trans_kind(in_fmt, TargetFmt::from_db(f)).is_ok()
        } else {
            flip_is_useful(in_fmt, &from, f)
        };
        if keep {
            add(&mut out, bound_pid, f);
        }
    }
    for p in others {
        let p_auto = p.format.trim().is_empty() || p.format.eq_ignore_ascii_case("auto");
        let pcands = formats_for(in_fmt, &p.format, Some(&p.protocol));
        let pfrom = pcands.first().cloned().unwrap_or_default();
        for (idx, f) in pcands.iter().enumerate() {
            let keep = if idx == 0 {
                true
            } else if p_auto {
                resolve_trans_kind(in_fmt, TargetFmt::from_db(f)).is_ok()
            } else {
                flip_is_useful(in_fmt, &pfrom, f)
            };
            if keep {
                add(&mut out, p.id, f);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prov(id: i64, fmt: &str) -> ReroutedProvider {
        prov_with(id, fmt, "")
    }

    /// 帶逐模型協議的來源。
    fn prov_with(id: i64, fmt: &str, protocol: &str) -> ReroutedProvider {
        ReroutedProvider {
            id,
            base_url: format!("http://127.0.0.1:{id}"),
            api_key: "k".into(),
            scheme: "bearer".into(),
            format: fmt.into(),
            protocol: protocol.into(),
        }
    }

    fn ids(v: &[Attempt]) -> Vec<(i64, String)> {
        v.iter()
            .map(|a| (a.provider_id, a.api_format.clone()))
            .collect()
    }

    /// Codex（Responses 入站）：來源宣告 chat 但模型只在 responses 上架 → 補一條直通候選。
    #[test]
    fn responses_inbound_gets_responses_fallback_for_chat_provider() {
        let a = plan_attempts(7, "openai-chat", None, InFmt::Responses, None, &[]);
        assert_eq!(
            ids(&a),
            vec![(7, "openai-chat".into()), (7, "openai-responses".into())]
        );
    }

    /// `mixed` 是「兩種端點都直通」，但 chat-only 的模型要換成 chat（翻譯那條）。
    #[test]
    fn mixed_provider_gets_chat_fallback_for_chat_only_models() {
        let a = plan_attempts(7, "mixed", None, InFmt::Responses, None, &[]);
        assert_eq!(
            ids(&a),
            vec![(7, "mixed".into()), (7, "openai-chat".into())]
        );
    }

    /// Claude Code（Anthropic 入站）：翻 responses 沒有翻譯器 → 不要排無效候選。
    #[test]
    fn anthropic_inbound_has_no_responses_fallback() {
        let a = plan_attempts(7, "openai-chat", None, InFmt::Anthropic, None, &[]);
        assert_eq!(ids(&a), vec![(7, "openai-chat".into())]);
    }

    /// 學到的協議排第一；與宣告相同時去重。
    #[test]
    fn learned_format_goes_first_and_dedupes() {
        let a = plan_attempts(7, "openai-chat", None, InFmt::Responses, Some("openai-responses"), &[]);
        assert_eq!(ids(&a)[0], (7, "openai-responses".into()));
        let b = plan_attempts(7, "openai-chat", None, InFmt::Responses, Some("openai-chat"), &[]);
        assert_eq!(
            ids(&b),
            vec![(7, "openai-chat".into()), (7, "openai-responses".into())]
        );
    }

    /// 其他來源排在綁定來源之後（來源換手是備援，不是首選）。
    #[test]
    fn other_providers_come_after_bound_one() {
        let others = vec![prov(3, "openai-chat"), prov(5, "openai-responses")];
        let a = plan_attempts(7, "openai-chat", None, InFmt::OpenAi, None, &others);
        // chat 入站翻 responses 是矩陣明定的錯誤（B6），所以只排各來源的宣告協議。
        assert_eq!(
            ids(&a),
            vec![
                (7, "openai-chat".into()),
                (3, "openai-chat".into()),
                (5, "openai-responses".into())
            ]
        );
    }

    // ───────── 逐模型協議與 auto（2026-10-10）─────────

    /// ★ 核心：**逐模型協議優先於來源宣告**。
    ///
    /// 這是「訂閱制上游單一來源、多種協議」的關鍵 —— opencode-go 宣告 `mixed`，
    /// 但 `claude-haiku-5-5` 實測只走 Anthropic Messages。
    #[test]
    fn per_model_protocol_wins_over_declared_format() {
        // Claude Code（Anthropic 入站）用那個模型 → 第一個候選就該是 anthropic
        let a = plan_attempts(7, "mixed", Some("anthropic"), InFmt::Anthropic, None, &[]);
        assert_eq!(ids(&a)[0], (7, "anthropic".into()), "{:?}", ids(&a));
        // 其餘候選：chat 可以（有 Anthropic→Chat 翻譯器），responses 不行
        assert!(ids(&a).contains(&(7, "openai-chat".into())));
        assert!(!ids(&a).contains(&(7, "openai-responses".into())));
    }

    /// ★ `auto` + Anthropic 入站：先試 anthropic（Claude Code 用 Claude 模型的路）。
    ///
    /// 舊行為下，mixed 來源的 Anthropic 請求一律被翻成 chat 送到
    /// `/chat/completions`，於是 Anthropic-only 的模型必定 400。
    #[test]
    fn auto_prefers_the_clients_own_protocol() {
        let a = plan_attempts(7, "auto", None, InFmt::Anthropic, None, &[]);
        assert_eq!(ids(&a)[0], (7, "anthropic".into()), "{:?}", ids(&a));
        assert!(ids(&a).contains(&(7, "openai-chat".into())));
        assert!(
            !ids(&a).contains(&(7, "openai-responses".into())),
            "Anthropic→Responses 沒有翻譯器，不該排無效候選：{:?}",
            ids(&a)
        );
    }

    /// `auto` + Codex（Responses 入站）：先直通 responses，再翻 chat，最後
    /// 才是 anthropic（2026-10-10 反向翻譯器實作後才排得進來）。
    #[test]
    fn auto_for_codex_prefers_responses_then_chat_then_anthropic() {
        let a = plan_attempts(7, "auto", None, InFmt::Responses, None, &[]);
        assert_eq!(
            ids(&a),
            vec![
                (7, "openai-responses".into()),
                (7, "openai-chat".into()),
                (7, "anthropic".into())
            ],
            "anthropic 排最後：它是兩跳，比直通與單跳貴"
        );
    }

    /// `auto` + OpenAI chat 入站：先 chat，再 anthropic
    /// （chat→responses 是矩陣明定的錯誤，不排）。
    #[test]
    fn auto_for_openai_client_schedules_chat_then_anthropic() {
        let a = plan_attempts(7, "auto", None, InFmt::OpenAi, None, &[]);
        assert_eq!(
            ids(&a),
            vec![(7, "openai-chat".into()), (7, "anthropic".into())],
            "{:?}",
            ids(&a)
        );
    }

    /// `auto` 不可以排出空清單（`flip_is_useful` 的第一邊會是 Err）。
    #[test]
    fn auto_never_yields_an_empty_plan() {
        for in_fmt in [InFmt::Anthropic, InFmt::Responses, InFmt::OpenAi] {
            let a = plan_attempts(7, "auto", None, in_fmt, None, &[]);
            assert!(!a.is_empty(), "auto + {in_fmt:?} 排出了空清單");
        }
    }

    /// 逐模型協議也適用於**其他來源**（跨來源換手時各自用自己的協議）。
    #[test]
    fn other_providers_use_their_own_model_protocol() {
        let others = vec![
            prov_with(3, "auto", "anthropic"),
            prov_with(5, "auto", "openai-chat"),
        ];
        let a = plan_attempts(7, "auto", None, InFmt::Anthropic, None, &others);
        let v = ids(&a);
        assert_eq!(v[0], (7, "anthropic".into()));
        // 綁定來源自己的 fallback 排在**其他來源之前**（來源換手是備援，不是首選）
        assert_eq!(v[1], (7, "openai-chat".into()), "{v:?}");
        // 來源 3 的模型是 anthropic-only → 它的第一候選是 anthropic
        let p3 = v.iter().position(|x| *x == (3, "anthropic".into()));
        assert!(p3.is_some(), "來源 3 應以 anthropic 開頭：{v:?}");
        // 來源 5 是 chat，且排在來源 3 之後
        let p5 = v.iter().position(|x| *x == (5, "openai-chat".into()));
        assert!(p5.is_some() && p5 > p3, "{v:?}");
    }

    /// 明確宣告 anthropic 的來源：另一邊只排 chat（Responses→Anthropic 沒翻譯器）。
    #[test]
    fn declared_anthropic_provider_falls_back_to_chat() {
        let a = plan_attempts(7, "anthropic", None, InFmt::Anthropic, None, &[]);
        assert_eq!(
            ids(&a),
            vec![(7, "anthropic".into()), (7, "openai-chat".into())],
            "{:?}",
            ids(&a)
        );
    }

    #[test]
    fn learned_protocol_persists_per_db_and_declared_format() {        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.db");
        let b = dir.path().join("b.db");
        crate::db::open_and_ensure(&a).unwrap();
        crate::db::open_and_ensure(&b).unwrap();
        let (pa, pb) = (a.as_path(), b.as_path());

        assert!(learned_format(pa, 1, "m", "openai-chat").is_none());
        learn_format(pa, 1, "M", "openai-chat", "openai-responses");
        // 大小寫不敏感
        assert_eq!(
            learned_format(pa, 1, "m", "openai-chat").as_deref(),
            Some("openai-responses")
        );
        // 別的來源、別的資料庫都看不到
        assert!(learned_format(pa, 2, "m", "openai-chat").is_none());
        assert!(learned_format(pb, 1, "m", "openai-chat").is_none());
        // ★ 宣告協議改了就是明確的設定變更 → 舊的學習結果不可以蓋掉它
        assert!(learned_format(pa, 1, "m", "openai-responses").is_none());
        // 同一個鍵再學一次會更新（後蓋前）
        learn_format(pa, 1, "m", "openai-chat", "mixed");
        assert_eq!(
            learned_format(pa, 1, "m", "openai-chat").as_deref(),
            Some("mixed")
        );
        // 空模型名／空協議不進庫
        learn_format(pa, 1, "  ", "openai-chat", "openai-chat");
        assert!(learned_format(pa, 1, "  ", "openai-chat").is_none());
    }

    /// 落庫的證據：新開一個連線（模擬網關重啟）仍讀得到。
    #[test]
    fn learned_protocol_survives_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("r.db");
        let conn0 = crate::db::open_and_ensure(&p).unwrap();
        // 學習結果一定屬於一個**存在的**來源：開庫時會清掉指向已刪除來源的孤兒列
        // （`crate::db::tests` 的 `open_purges_orphans_but_keeps_local_keys`）。
        conn0
            .execute(
                "INSERT OR IGNORE INTO providers (id, name, app_type, api_format, base_url,
                 api_key, auth_scheme, models_json, priority, enabled, created_at, updated_at)
                 VALUES (7,'p7','codex','openai-chat','http://127.0.0.1:1','k','bearer','[]',0,1,0,0)",
                [],
            )
            .unwrap();
        drop(conn0);
        let ps = p.as_path();
        learn_format(ps, 7, "grok-x", "openai-chat", "openai-responses");
        // 直接查表，確認是真的寫進 DB 而不是行程記憶體
        let conn = crate::db::open_and_ensure(&p).unwrap();
        let got: String = conn
            .query_row(
                "SELECT actual_format FROM provider_model_protocol
                 WHERE provider_id=7 AND model='grok-x' AND declared_format='openai-chat'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(got, "openai-responses");
        drop(conn);
        assert_eq!(
            learned_format(ps, 7, "grok-x", "openai-chat").as_deref(),
            Some("openai-responses")
        );
    }
}
