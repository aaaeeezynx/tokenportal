//! Claude Desktop 接管（對標 CC Switch 的 Claude Desktop 支援）。
//!
//! ## Claude Desktop 不是「換個 base_url」就能用
//!
//! Desktop 有一個 **fail-all validator**：`inferenceModels` 裡只要出現
//! 不像 Claude 模型的 ID，**整組 profile 都會被拒收**（一個模型都不剩）。
//! 白名單是 `claude-{sonnet,opus,haiku,fable}-*`（1.12603.1+ 加入 fable）。
//!
//! 所以真正的模型必須靠**兩段式映射**：
//!
//! ```text
//! Desktop 送 claude-sonnet-5
//!   → provider_models.display_name = "claude-sonnet-5"（本模組自動建立）
//!   → provider_models.actual_model = "deepseek-v4.1-flash"
//!   → models::resolve_actual() 在轉發前改寫 → 上游收到真的模型名
//! ```
//!
//! ## 與 CC Switch 共存（實測：這台機器已有它寫的 profile）
//!
//! `configLibrary/` 底下可以放多份 profile，用 `_meta.json` 的 `appliedId`
//! 決定哪一份生效。本模組**用自己的 ID 另寫一份**（尾碼是本專案埠 15722），
//! 只改 `appliedId` 與自己的 `entries` 條目 —— 不碰別人的 profile。
//! 還原時把自己那條移除，`appliedId` 指回剩下的第一條（通常是 CC Switch 的）。
//!
//! ## 為什麼 base_url 不像 CC Switch 那樣加 `/claude-desktop` 前綴
//!
//! 本專案的入站格式判定是**路徑後綴**（`proxy/matrix.rs`：
//! `path.ends_with("/messages")` → Anthropic）。Desktop 會打
//! `{base}/v1/messages`，所以裸的 `http://127.0.0.1:15722` 就直接命中，
//! 不需要額外前綴。

use std::path::{Path, PathBuf};

use rusqlite::Connection;
use serde::Serialize;
use serde_json::{json, Value};

use crate::fsutil;

/// 本專案在 `configLibrary` 裡的 profile ID。
///
/// 刻意與 CC Switch 的不同（它用 `…157210`，尾碼是它的埠 15721）。
pub const PROFILE_ID: &str = "00000000-0000-4000-8000-000000157220";
/// `_meta.json` 的 entries 裡顯示的名字。
const PROFILE_NAME: &str = "Token Portal";

/// Desktop 只認這幾種形狀的模型 ID（fail-all validator 的白名單）。
pub const ALIASES: [&str; 4] = [
    "claude-sonnet-5",
    "claude-opus-5",
    "claude-haiku-4-5",
    "claude-fable-5",
];

/// 一個別名目前的對映狀況。
#[derive(Debug, Clone, Serialize)]
pub struct AliasStatus {
    pub name: &'static str,
    /// 實際會送上游的模型名（None = 還沒建立對映）
    pub actual: Option<String>,
    /// 是哪個來源登記的
    pub provider: Option<String>,
}

/// 接管狀態。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopStatus {
    /// 找不找得到 Claude Desktop 的 configLibrary
    pub installed: bool,
    /// 我們自己那份 profile 是否為目前生效的（`_meta.json` 的 appliedId）
    pub configured: bool,
    pub profile_id: &'static str,
    pub config_library: String,
    pub profile_path: String,
    pub base_url: String,
    /// 可用來當 `inferenceGatewayApiKey` 的本地 Key 名稱（None = 沒有可用的）
    pub key_name: Option<String>,
    pub aliases: Vec<AliasStatus>,
    /// 給使用者看的一句話
    pub note: String,
}

// ---------------------------------------------------------------- 路徑 ---

fn local_app_data() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
}

/// `configLibrary` 目錄：優先 `Claude-3p`（Desktop 的第三方 profile 目錄），
/// 其次 `Claude`。兩者都可能存在 —— 實測這台機器兩個都有，而且只有
/// `Claude-3p` 底下有 `configLibrary`。
pub fn config_library() -> Option<PathBuf> {
    let lad = local_app_data()?;
    for dir in ["Claude-3p", "Claude"] {
        let p = lad.join(dir).join("configLibrary");
        if p.is_dir() {
            return Some(p);
        }
    }
    None
}

fn profile_path_at(lib: &Path) -> PathBuf {
    lib.join(format!("{PROFILE_ID}.json"))
}

fn meta_path_at(lib: &Path) -> PathBuf {
    lib.join("_meta.json")
}

mod meta;

pub(crate) use meta::{applied_is_us, meta_with_us, meta_without_us, profile_json};

// ------------------------------------------------------------ JSON 讀寫 ---

fn read_json(path: &Path) -> Value {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(|| json!({}))
}

fn write_json(path: &Path, v: &Value) -> Result<(), String> {
    let text = serde_json::to_string_pretty(v).map_err(|e| e.to_string())?;
    fsutil::atomic_write(path, format!("{text}\n").as_bytes())
        .map_err(|e| format!("寫入 {} 失敗：{e}", path.display()))
}

// ---------------------------------------------------------------- 資料庫 ---

/// 找出別名的對映現況（跨來源找，第一個優先）。
fn alias_status(conn: &Connection, alias: &str) -> (Option<String>, Option<String>) {
    conn.query_row(
        "SELECT m.actual_model, p.name
         FROM provider_models m JOIN providers p ON p.id = m.provider_id
         WHERE lower(m.display_name) = lower(?1)
         ORDER BY p.priority ASC, p.id ASC LIMIT 1",
        [alias],
        |r| Ok((r.get::<_, String>(0).ok(), r.get::<_, String>(1).ok())),
    )
    .ok()
    .map(|(a, p)| (a.filter(|s| !s.trim().is_empty()), p))
    .unwrap_or((None, None))
}

/// 選一個「有模型」的啟用來源，回傳 (provider_id, 第一個啟用模型).
fn pick_provider_and_model(conn: &Connection) -> Option<(i64, String)> {
    conn.query_row(
        "SELECT m.provider_id, COALESCE(NULLIF(m.actual_model,''), m.display_name)
         FROM provider_models m JOIN providers p ON p.id = m.provider_id
         WHERE m.enabled = 1 AND p.enabled = 1
         ORDER BY p.priority ASC, p.id ASC, m.ord ASC, m.id ASC
         LIMIT 1",
        [],
        |r| Ok((r.get(0)?, r.get::<_, String>(1)?)),
    )
    .ok()
}

/// 建立缺少的別名（已存在的不動 —— 使用者可能自己改過對映）。
fn ensure_aliases(conn: &Connection) -> Result<usize, String> {
    let Some((pid, actual)) = pick_provider_and_model(conn) else {
        return Err("沒有任何來源登記模型，請先在「上游來源」新增模型".to_string());
    };
    let mut created = 0;
    for alias in ALIASES {
        let (existing, _) = alias_status(conn, alias);
        if existing.is_some() {
            continue;
        }
        conn.execute(
            "INSERT INTO provider_models (provider_id, display_name, actual_model, ord, enabled)
             VALUES (?1, ?2, ?3, 99, 1)",
            rusqlite::params![pid, alias, actual],
        )
        .map_err(|e| format!("建立別名 {alias} 失敗：{e}"))?;
        created += 1;
    }
    Ok(created)
}

/// 找一個能當 Desktop 金鑰的本地 Key（不限工具，或含 claude）。
fn pick_key(conn: &Connection) -> Option<crate::keys::LocalKey> {
    let keys = crate::keys::list_keys(conn).ok()?;
    keys.into_iter().find(|k| {
        k.enabled
            && (k.allowed_apps.is_empty()
                || k.allowed_apps.iter().any(|a| a == "claude"))
    })
}

// ---------------------------------------------------------------- 命令 ---

/// 讀目前狀態（不寫任何檔案）。
pub fn status(conn: &Connection, port: u16) -> DesktopStatus {
    let lib = config_library();
    let installed = lib.is_some();
    let lib_s = lib
        .as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    let meta = lib
        .as_ref()
        .map(|l| read_json(&meta_path_at(l)))
        .unwrap_or_else(|| json!({}));
    let key = pick_key(conn);
    let aliases: Vec<AliasStatus> = ALIASES
        .iter()
        .map(|a| {
            let (actual, provider) = alias_status(conn, a);
            AliasStatus {
                name: a,
                actual,
                provider,
            }
        })
        .collect();
    let missing = aliases.iter().filter(|a| a.actual.is_none()).count();
    let note = if !installed {
        "找不到 Claude Desktop 的 configLibrary（本機可能沒安裝）".to_string()
    } else if key.is_none() {
        "沒有可用的本地 Key，請先到「本地 Key」建立一個".to_string()
    } else if missing > 0 {
        format!("有 {missing} 個 Claude 別名還沒建立對映，按「套用」會自動補")
    } else {
        format!("已就緒：Desktop 會看到 {} 個 Claude 模型", ALIASES.len())
    };
    DesktopStatus {
        installed,
        configured: applied_is_us(&meta),
        profile_id: PROFILE_ID,
        config_library: lib_s.clone(),
        profile_path: lib
            .as_ref()
            .map(|l| profile_path_at(l).display().to_string())
            .unwrap_or_default(),
        base_url: format!("http://127.0.0.1:{port}"),
        key_name: key.map(|k| k.name),
        aliases,
        note,
    }
}

/// 套用：建立缺少的別名 → 寫自己的 profile → 把 `_meta.json` 的 appliedId 指向自己。
///
/// 回傳「有沒有真的寫了東西」與一段說明。
pub fn apply(conn: &Connection, port: u16) -> Result<(DesktopStatus, String), String> {
    let lib = config_library().ok_or_else(|| {
        "找不到 Claude Desktop 的 configLibrary（本機可能沒安裝）".to_string()
    })?;
    let key = pick_key(conn).ok_or_else(|| {
        "沒有可用的本地 Key：請先到「本地 Key」建立一個（工具不限或含 claude）".to_string()
    })?;
    let secret = crate::keys::reveal_key(conn, key.id)?;
    let created = ensure_aliases(conn)?;

    let base = format!("http://127.0.0.1:{port}");
    let prof = profile_json(&base, &secret, &ALIASES);
    write_json(&profile_path_at(&lib), &prof)?;

    let meta_p = meta_path_at(&lib);
    let meta = read_json(&meta_p);
    let before_meta = meta.clone();
    let next = meta_with_us(&meta);
    if next != before_meta {
        write_json(&meta_p, &next)?;
    }

    let msg = format!(
        "已套用：寫入 profile（{} 個 Claude 別名{}），appliedId → {PROFILE_ID}",
        ALIASES.len(),
        if created > 0 {
            format!("，新建立 {created} 個對映")
        } else {
            String::new()
        }
    );
    Ok((status(conn, port), msg))
}

/// 還原：刪自己的 profile、把自己從 `_meta.json` 移除、appliedId 指回其他條目。
pub fn restore(conn: &Connection, port: u16) -> Result<(DesktopStatus, String), String> {
    let lib = config_library().ok_or_else(|| "找不到 Claude Desktop 的 configLibrary".to_string())?;
    let prof = profile_path_at(&lib);
    let mut removed_profile = false;
    if prof.exists() {
        std::fs::remove_file(&prof).map_err(|e| format!("刪除 profile 失敗：{e}"))?;
        removed_profile = true;
    }
    let meta_p = meta_path_at(&lib);
    let meta = read_json(&meta_p);
    let next = meta_without_us(&meta);
    if next != meta {
        write_json(&meta_p, &next)?;
    }
    let back_to = next
        .get("appliedId")
        .and_then(|v| v.as_str())
        .unwrap_or("（沒有其他 profile）")
        .to_string();
    let msg = format!(
        "{}；appliedId → {back_to}",
        if removed_profile {
            "已刪除 Token Portal 的 profile"
        } else {
            "本來就沒有 Token Portal 的 profile"
        }
    );
    Ok((status(conn, port), msg))
}
