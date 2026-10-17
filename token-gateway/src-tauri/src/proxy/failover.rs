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
        // Gemini 沒有「另一邊」：`(Gemini, 其他)` 全是矩陣明定的錯誤
        //（`E_GEMINI_IN_ONLY`）。要讓 OpenAI 系客戶端用 Gemini 模型，得先有
        // OpenAI ⇄ Gemini 的翻譯器 —— 那是 2026-10-11 排定的下一段工作。
        "gemini" => &[],
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
/// ——客戶端自己用的協議，通常就是這個模型支援的協議 ——再試其餘幾種。
/// 這個順序讓「一個來源服務多種客戶端」在第一次請求就大概率命中，
/// 猜錯也只是一次失敗往返，之後由 `learn_format` 記住。
fn formats_for(in_fmt: InFmt, declared: &str, model_protocol: Option<&str>) -> Vec<String> {
    let explicit = model_protocol
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .unwrap_or_else(|| declared.trim());
    if explicit.is_empty() || explicit.eq_ignore_ascii_case("auto") {
        // **Gemini 入站必須排 gemini 第一**：Gemini 原生請求只有
        // `(Gemini, Gemini)` 這一格是通的，其餘全是 `E_GEMINI_IN_ONLY`。
        // 舊版把 Gemini 落進 `_ => "openai-chat"`，於是 Gemini 客戶端在
        // `auto` 來源上**第一個候選就被擋下** —— 而 `plan_attempts` 對第一個
        // 候選是無條件排入的，那個錯誤會直接回給使用者。即使模型其實是
        // Gemini 原生的也一樣。2026-10-11 修。
        let primary = match in_fmt {
            InFmt::Anthropic => "anthropic",
            InFmt::Responses => "openai-responses",
            InFmt::OpenAi => "openai-chat",
            InFmt::Gemini => "gemini",
            // 中性路徑（/v1/models 之類）：不猜，照 chat 先試
            _ => "openai-chat",
        };
        let mut v = vec![primary.to_string()];
        // 其餘協議的順序固定；用不到的會被 `plan_attempts` 的矩陣過濾丟掉
        //（Gemini 入站時 openai-chat 是 Err，反之亦然）。
        for p in ["openai-chat", "openai-responses", "anthropic", "gemini"] {
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
