//! 測試子模組的入口與共用前置。
//!
//! 原本所有測試都擠在這一個檔案（2,025 行），已依主題拆成六個子模組。
//! 這裡保留前置並以 `#[path]` 引入子模組，讓 **`proxy::tests::<name>` 這個
//! 路徑完全不變** —— 既有測試名稱、`cargo test` 的篩選字串、以及 `#[ignore]`
//! 的執行方式都不必改。
//!
//! 子模組住在更深的層級，`use super::*;` 只會帶進**父模組自己的**綁定，不會
//! 帶進父模組的 `use` 別名（別名是私有的）。所以子模組需要的型別與常數一律
//! 用 `pub(super) use` 再匯出 —— 同一個名字只寫一次，避免 E0252。

#![allow(unused_imports)]

use super::*;
use axum::http::HeaderValue;

// 子模組真正需要的東西，以 `pub(super)` 再匯出（見上方說明）：
// - `SseUsage`：原本經由 `proxy.rs` 的檔案層級 `use crate::translate::{…}` 取得，
//   該行隨 `extract_usage` 搬進 `proxy/logging.rs`，故在此明確引入。
// - `E_*`：矩陣的 400 訊息常數住在 `proxy::matrix`（第十一步拆出）。
pub(super) use crate::proxy::matrix::{
    E_ANTHROPIC_UNSUPPORTED, E_CHAT_TO_RESPONSES, E_GEMINI_IN_ONLY, E_GEMINI_OUT_ONLY,
};
pub(super) use crate::translate::SseUsage;

// ------------------------------------------------------------ 共用 helper ---
// e2e 子模組都要用（`e2e_strict` 與 `e2e_translate` 都有跨格式的案例），
// 所以放在這個共同父模組，而不是某一個子模組裡。

/// 插一個來源（base_url 先留佔位，測試再 UPDATE 成假上游的埠）。
pub(super) fn seed_provider(conn: &rusqlite::Connection, name: &str) -> i64 {
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

/// 插一個可用模型；`reasoning` 給 `None` 代表留空（未宣告）。
pub(super) fn seed_model(
    conn: &rusqlite::Connection,
    pid: i64,
    actual: &str,
    reasoning: Option<&str>,
) {
    conn.execute(
        "INSERT INTO provider_models
             (provider_id, display_name, actual_model, reasoning, ord, enabled)
         VALUES (?1, ?2, ?2, ?3, 0, 1)",
        rusqlite::params![pid, actual, reasoning.unwrap_or("")],
    )
    .unwrap();
}

/// 簽一把綁這個來源的本地 Key。
pub(super) fn seed_key(conn: &rusqlite::Connection, pid: i64, name: &str) -> String {
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

#[path = "tests/unit.rs"]
mod unit;
#[path = "tests/strip.rs"]
mod strip;
#[path = "tests/matrix.rs"]
mod matrix;
#[path = "tests/e2e_basic.rs"]
mod e2e_basic;
#[path = "tests/e2e_translate.rs"]
mod e2e_translate;
#[path = "tests/e2e_strict.rs"]
mod e2e_strict;
