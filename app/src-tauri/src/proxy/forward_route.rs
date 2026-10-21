//! 模型 → 來源解析（從 forward.rs 抽出，見該檔開頭）。
//!
//! 這些函式的共同主題：**「這個模型名該去哪個來源」**。它跟 forward.rs 其餘
//! 部分（組裝上游目標、轉譯請求體、剝離欄位）是不同關注點，抽出來之後
//! forward.rs 也回到 400 行以內。
//!
//! 匹配規則是 display_name 或 actual_model 任一命中（見 db/schema.rs 的
//! provider_models）—— 這是模型映射與跨來源路由的接點。

use super::*;

/// 取渠道顯示名（僅用於錯誤訊息；查不到時給可讀的佔位字串）。
pub(super) fn provider_name(conn: &rusqlite::Connection, provider_id: i64) -> String {
    conn.query_row("SELECT name FROM providers WHERE id=?1", [provider_id], |r| {
        r.get(0)
    })
    .unwrap_or_else(|_| "（未知來源）".to_string())
}

/// 同一個上游 Key 登記在多個來源時，列出所有來源名（僅用於錯誤訊息）。
pub(super) fn same_key_provider_names(conn: &rusqlite::Connection, api_key: &str) -> Vec<String> {
    if api_key.is_empty() {
        return Vec::new();
    }
    let mut stmt = match conn.prepare(
        "SELECT name FROM providers
         WHERE enabled = 1 AND api_key = ?1
         ORDER BY priority ASC, id ASC",
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let names = match stmt.query_map([api_key], |r| r.get::<_, String>(0)) {
        Ok(it) => it.filter_map(Result::ok).collect(),
        Err(_) => Vec::new(),
    };
    names
}

fn rerouted_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<ReroutedProvider> {
    Ok(ReroutedProvider {
        id: r.get(0)?,
        base_url: r.get(1)?,
        api_key: r.get(2)?,
        scheme: r.get(3)?,
        format: r.get(4)?,
        protocol: r.get(5)?,
        prefixes: r.get(6)?,
    })
}

/// 跨來源模型路由：在其他啟用來源的可用模型表中找 display/actual 命中
///（排除 Key 綁定來源，按來源優先級、id 取第一個）。
pub(super) fn resolve_model_provider(
    conn: &rusqlite::Connection,
    model: &str,
    exclude_pid: i64,
) -> Option<ReroutedProvider> {
    conn.query_row(
        "SELECT p.id, p.base_url, p.api_key, p.auth_scheme, p.api_format, m.protocol,
                p.protocol_prefixes
         FROM provider_models m JOIN providers p ON p.id = m.provider_id
         WHERE m.enabled = 1 AND p.enabled = 1 AND p.id != ?2
           AND (lower(m.display_name) = lower(?1) OR lower(m.actual_model) = lower(?1))
         ORDER BY p.priority ASC, p.id ASC LIMIT 1",
        rusqlite::params![model, exclude_pid],
        rerouted_from_row,
    )
    .ok()
}

/// 直連模式的同 Key 消歧：只在**共用同一把上游 Key** 的啟用來源中，
/// 找登記了 `model` 的來源。不可退化成任意跨來源路由，否則會拿客戶端
/// 沒送來的另一把上游 Key 發請求。
pub(super) fn resolve_model_provider_by_key(
    conn: &rusqlite::Connection,
    model: &str,
    api_key: &str,
) -> Option<ReroutedProvider> {
    if api_key.is_empty() {
        return None;
    }
    conn.query_row(
        "SELECT p.id, p.base_url, p.api_key, p.auth_scheme, p.api_format, m.protocol,
                p.protocol_prefixes
         FROM provider_models m JOIN providers p ON p.id = m.provider_id
         WHERE m.enabled = 1 AND p.enabled = 1 AND p.api_key = ?2
           AND (lower(m.display_name) = lower(?1) OR lower(m.actual_model) = lower(?1))
         ORDER BY p.priority ASC, p.id ASC LIMIT 1",
        rusqlite::params![model, api_key],
        rerouted_from_row,
    )
    .ok()
}

/// 跨來源模型路由（**多候選**版）：其他啟用來源中登記了同一個模型的全部來源，
/// 依來源優先級排序（同一來源只出現一次）。
///
/// `resolve_model_provider` 取第一名；這裡取整條佇列，讓 `failover.rs` 在第一名
/// 失敗（連不上、5xx、協議不支援）時還能往下換 —— 這是 cc-switch「failover queue」
/// 的精神。失敗換手只發生在**已經決定要轉發**之後，白名單與直連語意不變。
pub(super) fn resolve_model_providers(
    conn: &rusqlite::Connection,
    model: &str,
    exclude_pid: i64,
) -> Vec<ReroutedProvider> {
    let mut stmt = match conn.prepare(
        "SELECT p.id, p.base_url, p.api_key, p.auth_scheme, p.api_format, m.protocol,
                p.protocol_prefixes
         FROM provider_models m JOIN providers p ON p.id = m.provider_id
         WHERE m.enabled = 1 AND p.enabled = 1 AND p.id != ?2
           AND (lower(m.display_name) = lower(?1) OR lower(m.actual_model) = lower(?1))
         GROUP BY p.id
         ORDER BY p.priority ASC, p.id ASC",
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let rows = stmt.query_map(rusqlite::params![model, exclude_pid], rerouted_from_row);
    match rows {
        Ok(it) => it.filter_map(Result::ok).collect(),
        Err(_) => Vec::new(),
    }
}
