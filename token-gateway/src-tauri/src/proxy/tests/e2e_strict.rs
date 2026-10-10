//! 端到端：嚴格上游、剝離重試、診斷可見性
//!
//! 由 `proxy/tests.rs` 拆分而來（原檔 2,025 行）。測試**內容與斷言一字
//! 未改**，只搬位置；對外仍經由 `proxy::tests::` 這個路徑被呼叫。
//!
//! 模組佈局：本檔只留前置與子模組宣告；測試依主題放在
//! `e2e_strict/strip_retry.rs`（剝離後重發）、
//! `e2e_strict/strict_content.rs`（上游拒絕空 content）、
//! `e2e_strict/diagnostics.rs`（拒絕請求的診斷可見性）。

    // 前置由父模組 `proxy::tests` 的 `pub(super) use` 再匯出提供（含 `SseUsage`
    // 與 `E_*` 常數）；`use super::*;` 會把它們一併帶進來。這裡**不可**再寫
    // `use super::matrix::…` —— 在子模組裡 `super` 是 `proxy::tests`，而那裡
    // 的 `matrix` 指的是測試子模組本身。
    use super::*;
    use axum::http::HeaderValue;

    #[path = "e2e_strict/strip_retry.rs"]
    mod strip_retry;
    #[path = "e2e_strict/strict_content.rs"]
    mod strict_content;
    #[path = "e2e_strict/diagnostics.rs"]
    mod diagnostics;
    #[path = "e2e_strict/failover.rs"]
    mod failover;
    // 被動學習：上游拒收 reasoning_effort 時記下該強度並重試
    #[path = "e2e_strict/reasoning_learn.rs"]
    mod reasoning_learn;
    // 被動學習：上游拒收推理簽章時剝掉重試並記住
    #[path = "e2e_strict/encrypted_learn.rs"]
    mod encrypted_learn;

    // ------------------------------------------------------ 共用 helper ---
    // 子模組以 `use super::*;` 取用（本檔的項目對它們是可見的）。

    /// 插一個來源（base_url 先留佔位，測試再 UPDATE 成假上游的埠）。
    fn seed_provider(conn: &rusqlite::Connection, name: &str) -> i64 {
        conn.execute(
            "INSERT INTO providers (name, app_type, api_format, base_url, api_key, auth_scheme,
             models_json, priority, enabled, created_at, updated_at)
             VALUES (?1,'codex','openai-chat','PLACEHOLDER','up-key','bearer','[]',0,1,0,0)",
            [name],
        )
        .unwrap();
        conn.query_row("SELECT id FROM providers WHERE name=?1", [name], |r| r.get(0))
            .unwrap()
    }

    /// 插一個可用模型。
    fn seed_model(conn: &rusqlite::Connection, pid: i64, actual: &str, reasoning: Option<&str>) {
        conn.execute(
            "INSERT INTO provider_models
                 (provider_id, display_name, actual_model, reasoning, ord, enabled)
             VALUES (?1, ?2, ?2, ?3, 0, 1)",
            rusqlite::params![pid, actual, reasoning.unwrap_or("")],
        )
        .unwrap();
    }

    /// 簽一把綁這個來源的本地 Key。
    fn seed_key(conn: &rusqlite::Connection, pid: i64, name: &str) -> String {
        keys::create_key(
            conn,
            &keys::KeyInput {
                name: name.into(),
                provider_id: Some(pid),
                quota_tokens: -1,
                rate_limit_qpm: 0,
                allowed_models: vec![],
                allowed_apps: vec![],
                expires_at: None,
            },
        )
        .unwrap()
        .secret
    }
