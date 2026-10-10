//! 「這個模型的這個思考強度，上游收不收」的記憶。
//!
//! ## 為什麼需要這張表
//!
//! 上游**不公開**每個模型支援哪些 `reasoning_effort`：OpenCode 的 `/models`
//! 只回 `id`／`object`／`created`／`owned_by`，官方文件也沒有；而送錯值時上游
//! 回的是泛用的 `400 Invalid request parameters`，與其他參數錯誤無法區分。
//! 所以只能實測，並把結果記下來。
//!
//! 2026-10-08 的實例：MiMo-V2.6-Pro／Flash 吃 `low`／`medium`／`high`，
//! 但 `minimal`／`xhigh`／`max`／`ultra`／`persistent` 一律 400。
//! 舊版沒有這層記憶，使用者把 `xhigh` 填進宣告、目錄就把它提供給 Codex，
//! 選到就 400。
//!
//! ## 與 `provider_models.reasoning` 的分工
//!
//! - `provider_models.reasoning`：**使用者的宣告**（我想讓這個模型用哪些強度）
//! - 本表：**上游的事實**（我實測到哪些真的能過）
//!
//! 目錄生成時取兩者的交集。**被動學習只寫本表、不動使用者的宣告** ——
//! 使用者的設定不該被背景行為悄悄改掉。

use rusqlite::Connection;
use serde::Serialize;

/// 探測用的候選強度（**單一真相來源**）。
///
/// 前 7 個是 App 長年提供的值；`ultra`／`persistent` 是 Codex 生態真實存在、
/// 且實測確實會被送到上游的值（見 `models/probe.rs`）。
/// 不含 `unset` —— 「不帶 reasoning_effort」由 [`LEVEL_NONE`] 單獨代表。
pub(crate) const CANDIDATE_LEVELS: [&str; 8] = [
    "minimal", "low", "medium", "high", "xhigh", "max", "ultra", "persistent",
];

/// 「這一筆沒有帶 reasoning_effort」的哨兵值。
pub(crate) const LEVEL_NONE: &str = "__none__";

/// 記憶的一列。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ReasoningFact {
    pub level: String,
    pub supported: bool,
    /// `probe`（實測，權威）或 `learned`（從真實請求的失敗順手學到）
    pub source: String,
    pub checked_at: i64,
}

/// 寫入一筆事實（同鍵覆蓋；`probe` 的結果會蓋掉 `learned` 的猜測）。
///
/// 呼叫端多半用短命連線（見 `proxy/retry.rs` 的同類作法），失敗不致命。
pub(crate) fn record(
    conn: &Connection,
    provider_id: i64,
    model: &str,
    level: &str,
    supported: bool,
    source: &str,
    now_ms: i64,
) -> rusqlite::Result<()> {
    if model.trim().is_empty() || level.trim().is_empty() {
        return Ok(());
    }
    conn.execute(
        "INSERT INTO provider_model_reasoning
             (provider_id, model, level, supported, source, checked_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(provider_id, model, level)
         DO UPDATE SET supported  = excluded.supported,
                       source     = excluded.source,
                       checked_at = excluded.checked_at",
        rusqlite::params![
            provider_id,
            model.trim(),
            level.trim(),
            if supported { 1 } else { 0 },
            source,
            now_ms
        ],
    )?;
    Ok(())
}

/// 讀取某來源某模型的全部事實（沒有就空）。
pub(crate) fn facts(
    conn: &Connection,
    provider_id: i64,
    model: &str,
) -> Vec<ReasoningFact> {
    let Ok(mut stmt) = conn.prepare(
        "SELECT level, supported, source, checked_at
         FROM provider_model_reasoning
         WHERE provider_id = ?1 AND model = ?2",
    ) else {
        return Vec::new();
    };
    let rows = stmt.query_map(rusqlite::params![provider_id, model.trim()], |r| {
        Ok(ReasoningFact {
            level: r.get(0)?,
            supported: r.get::<_, i64>(1)? != 0,
            source: r.get(2)?,
            checked_at: r.get(3)?,
        })
    });
    match rows {
        Ok(it) => it.filter_map(Result::ok).collect(),
        Err(_) => Vec::new(),
    }
}

/// 這一組候選強度裡，**已知支援**的有哪些（只回「有紀錄且支援」的）。
///
/// 刻意不回「未知」—— 呼叫端要的是「確定可用」的清單。
pub(crate) fn known_supported(
    conn: &Connection,
    provider_id: i64,
    model: &str,
) -> Vec<String> {
    let mut v: Vec<String> = facts(conn, provider_id, model)
        .into_iter()
        .filter(|f| f.supported && f.level != LEVEL_NONE)
        .map(|f| f.level)
        .collect();
    v.sort_by_key(|l| level_rank(l));
    v.dedup();
    v
}

/// 這一組候選強度裡，**已知不支援**的有哪些。
pub(crate) fn known_unsupported(
    conn: &Connection,
    provider_id: i64,
    model: &str,
) -> Vec<String> {
    facts(conn, provider_id, model)
        .into_iter()
        .filter(|f| !f.supported)
        .map(|f| f.level)
        .collect()
}

/// 候選清單裡的規範順序（未知的排最後，保持穩定）。
pub(crate) fn level_rank(level: &str) -> usize {
    CANDIDATE_LEVELS
        .iter()
        .position(|l| *l == level)
        .unwrap_or(CANDIDATE_LEVELS.len())
}

/// 完全沒有資訊時，翻譯器願意轉發的保守預設。
///
/// **刻意不含 `xhigh`／`max`**：這兩個在多個上游（實測 opencode-go 的兩個
/// MiMo）都會被 400。舊版的硬編碼白名單正好相反 —— 它轉發唯二會失敗的兩個、
/// 丟掉其餘全部，是 2026-10-08 那個 400 的直接原因。
pub(crate) const DEFAULT_EFFORTS: [&str; 3] = ["low", "medium", "high"];

/// 這個模型有沒有任何紀錄（＝有沒有被看過）。
///
/// **不能只看「支援集合空不空」**：一個「什麼強度都不收」的模型，支援集合
/// 正好是空的，但那與「從沒測過」是完全不同的兩件事 —— 前者目錄該宣告成
/// 空陣列（Codex 就不提供強度選項），後者才該退回模板並集。
pub(crate) fn has_facts(conn: &Connection, provider_id: i64, model: &str) -> bool {
    conn.query_row(
        "SELECT 1 FROM provider_model_reasoning
         WHERE provider_id = ?1 AND model = ?2 LIMIT 1",
        rusqlite::params![provider_id, model.trim()],
        |r| r.get::<_, i64>(0),
    )
    .is_ok()
}

/// 翻譯器可以轉發的強度：**上游收得下的那些**。
///
/// 三段判定：
/// 1. 使用者有宣告 → 以宣告為準
/// 2. 沒宣告但有實測記憶 → 用實測支援集合（**可能是空的**：該模型不收任何強度）
/// 3. 兩者都沒有 → [`DEFAULT_EFFORTS`]
///
/// 最後一律扣掉「已知不支援」。所以即使目錄宣告錯了（例如模型沒偵測過、
/// 而 Codex 送了 `xhigh`），翻譯層仍會把它擋下來而不是送去撞 400。
pub(crate) fn allowed_efforts(conn: &Connection, provider_id: i64, model: &str) -> Vec<String> {
    let declared: Vec<String> = conn
        .query_row(
            "SELECT reasoning FROM provider_models
             WHERE provider_id = ?1 AND actual_model = ?2 LIMIT 1",
            rusqlite::params![provider_id, model.trim()],
            |r| r.get::<_, String>(0),
        )
        .unwrap_or_default()
        .split(',')
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty() && s != "unset")
        .collect();

    let supported = known_supported(conn, provider_id, model);
    let unsupported = known_unsupported(conn, provider_id, model);

    let mut base: Vec<String> = if !declared.is_empty() {
        declared
    } else if has_facts(conn, provider_id, model) {
        // 測過了：以實測為準（空就是空，不退回猜測）
        supported
    } else {
        DEFAULT_EFFORTS.iter().map(|s| s.to_string()).collect()
    };
    base.retain(|l| !unsupported.iter().any(|u| u == l));
    base.sort_by_key(|l| level_rank(l));
    base.dedup();
    base
}

