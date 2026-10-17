//! `proxy::failover` 的候選排序測試。
//!
//! 從 `failover.rs` 的內嵌測試模組搬出來（2026-10-11）：加入 Gemini 的
//! 案例之後那支超過專案 400 行的自律上限。測試內容一字未改。

use crate::proxy::failover::*;
use crate::proxy::forward::ReroutedProvider;
use crate::proxy::matrix::InFmt;

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
    for in_fmt in [
        InFmt::Anthropic,
        InFmt::Responses,
        InFmt::OpenAi,
        InFmt::Gemini,
    ] {
        let a = plan_attempts(7, "auto", None, in_fmt, None, &[]);
        assert!(!a.is_empty(), "auto + {in_fmt:?} 排出了空清單");
    }
}

// ───────── Gemini（2026-10-11）─────────

/// ★ 迴歸：**Gemini 入站 ＋ auto → 第一個候選必須是 `gemini`**。
///
/// 舊版把 Gemini 落進 `_ => "openai-chat"`，而 `(Gemini, OpenAiChat)` 是矩陣
/// 明定的 `E_GEMINI_IN_ONLY`；偏偏 `plan_attempts` 對第一個候選是**無條件**
/// 排入的，所以 Gemini 客戶端在 `auto` 來源上第一個候選就被擋下 ——
/// 即使那個模型其實是 Gemini 原生的。
#[test]
fn auto_for_gemini_client_schedules_gemini_first() {
    let a = plan_attempts(7, "auto", None, InFmt::Gemini, None, &[]);
    assert_eq!(ids(&a)[0], (7, "gemini".into()), "{:?}", ids(&a));
    // Gemini 入站時其餘協議全是矩陣的 Err，不該被排進來
    assert_eq!(ids(&a), vec![(7, "gemini".into())], "{:?}", ids(&a));
}

/// 逐模型協議設成 `gemini`：Gemini 客戶端照它走。
#[test]
fn per_model_gemini_protocol_is_honoured() {
    let a = plan_attempts(7, "auto", Some("gemini"), InFmt::Gemini, None, &[]);
    assert_eq!(ids(&a)[0], (7, "gemini".into()), "{:?}", ids(&a));
}

/// 反過來：**OpenAI 客戶端**碰到 gemini-only 的模型 → 只剩那一個候選，
/// 由矩陣回明確的 `E_GEMINI_OUT_ONLY`（等 ② 的翻譯器做好才會多排）。
#[test]
fn gemini_only_model_gives_openai_clients_a_clear_error_not_a_silent_retry() {
    let a = plan_attempts(7, "auto", Some("gemini"), InFmt::OpenAi, None, &[]);
    assert_eq!(
        ids(&a),
        vec![(7, "gemini".into())],
        "不該再多排別的候選（gemini 沒有『另一邊』）：{:?}",
        ids(&a)
    );
}

/// 明確宣告 `gemini` 的來源：`other_sides` 是空的（Gemini 沒有另一邊）。
#[test]
fn declared_gemini_provider_has_no_other_side() {
    let a = plan_attempts(7, "gemini", None, InFmt::Gemini, None, &[]);
    assert_eq!(ids(&a), vec![(7, "gemini".into())], "{:?}", ids(&a));
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
