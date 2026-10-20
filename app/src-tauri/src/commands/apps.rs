//! 各本機工具的接管、還原與版本命令（原 `commands.rs` 的工具段）。

use tauri::{AppHandle, Manager, State};

use crate::db::DbState;
use crate::tools;

#[tauri::command]
pub fn tools_detect(port: u16) -> Vec<tools::ToolStatus> {
    tools::detect_tools(port)
}

#[tauri::command]
pub async fn tool_versions() -> Vec<tools::ToolVersion> {
    tauri::async_runtime::spawn_blocking(tools::tool_versions)
        .await
        .unwrap_or_default()
}

#[tauri::command]
pub async fn tool_latest() -> Vec<tools::ToolLatest> {
    tauri::async_runtime::spawn_blocking(tools::tool_latest)
        .await
        .unwrap_or_default()
}

#[tauri::command]
pub async fn tool_update(app: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || tools::tool_update_run(&app))
        .await
        .map_err(|e| e.to_string())?
}

// ---- 安裝／全部升級／重複安裝診斷（見 tools/versions/doctor.rs）----

/// 一鍵安裝（npm 全域）；只給尚未安裝的工具用。
#[tauri::command]
pub async fn tool_install(app: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || tools::tool_install_run(&app))
        .await
        .map_err(|e| e.to_string())?
}

/// 全部升級：只處理「已安裝且確實落後」的工具，逐筆回報結果。
#[tauri::command]
pub async fn tool_upgrade_all() -> Vec<tools::UpdateOutcome> {
    tauri::async_runtime::spawn_blocking(tools::tool_update_all)
        .await
        .unwrap_or_default()
}

/// 重複安裝診斷：同一個工具在不同目錄各有一份時，「跑的那份」可能不是剛更新的那份。
#[tauri::command]
pub async fn tool_duplicates() -> Vec<tools::DuplicateReport> {
    tauri::async_runtime::spawn_blocking(tools::tool_duplicates)
        .await
        .unwrap_or_default()
}

fn normalize_switch_request(
    db: &DbState,
    req: &mut tools::SwitchRequest,
    port: u16,
) -> Result<(), String> {
    // base_url 一律指向本網關（唯一模式；`via_gateway` 已於 2026-09-28 移除）。
    req.base_url = tools::gateway_url(port, &req.app);
    if let Some(pid) = req.provider_id {
        let conn = db.conn.lock().map_err(|e| e.to_string())?;
        let api_format: Option<String> = conn
            .query_row(
                "SELECT api_format FROM providers WHERE id=?1 AND enabled=1",
                rusqlite::params![pid],
                |r| r.get(0),
            )
            .ok();
        drop(conn);
        let Some(api_format) = api_format else {
            return Err("該來源不存在或已停用（可能已被刪除），請重新選擇來源接管".into());
        };
        req.provider_format = Some(api_format);
    }
    Ok(())
}

#[tauri::command]
pub fn switch_plan(
    db: State<DbState>,
    mut req: tools::SwitchRequest,
    port: u16,
) -> Result<tools::SwitchPlan, String> {
    normalize_switch_request(&db, &mut req, port)?;
    tools::plan_switch(&req, port)
}

#[tauri::command]
pub fn switch_apply(
    app: AppHandle,
    db: State<DbState>,
    mut req: tools::SwitchRequest,
    port: u16,
) -> Result<tools::SwitchResult, String> {
    let data = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?;
    normalize_switch_request(&db, &mut req, port)?;
    // 直連上游模式：Key 欄留空 → 以所選來源的上游 Key 直連（不經本地 Key）。
    // 此處從庫中取出上游 Key 注入請求；config 側明文寫入（計劃頁已警告）。
    // 對所有工具一致：claude 寫入自家配置，codex/opencode 寫 provider 段。
    if req.direct_upstream {
        let pid = req.provider_id.ok_or("直連模式需先選擇來源")?;
        let conn = db.conn.lock().map_err(|e| e.to_string())?;
        let up: String = conn
            .query_row(
                "SELECT api_key FROM providers WHERE id=?1 AND enabled=1",
                rusqlite::params![pid],
                |r| r.get(0),
            )
            .map_err(|_| "直連模式需該來源已啟用".to_string())?;
        if up.trim().is_empty() {
            return Err("該來源未填寫上游 API Key，無法直連（請先在來源詳情填寫）".to_string());
        }
        req.api_key = up;
    }
    // Codex 模型目錄按需產生（模板收編自網關自持副本，不再依賴 cc-switch）
    let catalog = if req.gen_catalog && req.app == "codex" {
        let pid = req
            .provider_id
            .ok_or("產生模型目錄需先選擇渠道")?;
        let home = tools::user_home()?;
        let conn = db.conn.lock().map_err(|e| e.to_string())?;
        Some(
            crate::models::codex_catalog_generate(&conn, &home, &data, pid, req.catalog_union)
                .map(|p| p.to_string_lossy().to_string())?,
        )
    } else {
        None
    };
    tools::apply_switch(&data, req, port, catalog)
}

#[tauri::command]
pub fn switch_restore(
    app: AppHandle,
    app_name: String,
    port: u16,
) -> Result<String, String> {
    let data = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?;
    tools::restore_backup_port(&data, &app_name, port)
}

/// 「關閉網關」：忠實還原接管前備份，**再**把來源換回該工具的第一方
/// （Codex→OpenAI/GPT、Claude Code→Claude、OpenCode→自己的登入）。
///
/// 與 [`switch_restore`] 的差別：那支只還原備份，忠實到連「兩天前的舊快照」
/// 都照寫 —— 使用者關掉網關後會卡在一個早就不用的第三方來源上。詳見
/// `tools/native.rs` 的模組註解。
#[tauri::command]
pub fn switch_off(
    app: AppHandle,
    app_name: String,
    port: u16,
) -> Result<String, String> {
    let data = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?;
    tools::restore_native_port(&data, &app_name, port)
}

#[tauri::command]
pub fn codex_doctor(port: u16) -> Vec<String> {
    tools::codex_doctor(port)
}

// ---- Claude Desktop 接管（見 claude_desktop.rs）----
//
// 這三個都要碰 DB，而 `State` 不能跨執行緒；沿用 `catalog_fetch` 的寫法：
// 在 `spawn_blocking` 裡用 `AppHandle` 重新取 state。

/// 讀 Claude Desktop 接管狀態（不寫任何檔案）。
#[tauri::command]
pub async fn desktop_status(
    app: AppHandle,
    port: u16,
) -> Result<crate::claude_desktop::DesktopStatus, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let db = app.state::<DbState>();
        let conn = db.conn.lock().map_err(|e| e.to_string())?;
        Ok(crate::claude_desktop::status(&conn, port))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 套用：建立缺少的 Claude 別名 → 寫自己的 profile → appliedId 指向自己。
#[tauri::command]
pub async fn desktop_apply(
    app: AppHandle,
    port: u16,
) -> Result<(crate::claude_desktop::DesktopStatus, String), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let db = app.state::<DbState>();
        let conn = db.conn.lock().map_err(|e| e.to_string())?;
        crate::claude_desktop::apply(&conn, port)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 還原：刪自己的 profile、從 `_meta.json` 移除、appliedId 指回其他條目。
#[tauri::command]
pub async fn desktop_restore(
    app: AppHandle,
    port: u16,
) -> Result<(crate::claude_desktop::DesktopStatus, String), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let db = app.state::<DbState>();
        let conn = db.conn.lock().map_err(|e| e.to_string())?;
        crate::claude_desktop::restore(&conn, port)
    })
    .await
    .map_err(|e| e.to_string())?
}
