//! 模型目錄與價格識別命令（原 `commands.rs` 的「模型」與「價格識別」段）。

use tauri::{AppHandle, Manager, State};

use crate::db::DbState;

// ---------------------------------------------------------------- 模型 ---

#[tauri::command]
pub async fn catalog_fetch(
    app: AppHandle,
    provider_id: i64,
    api_key: Option<String>,
    base_url: Option<String>,
) -> Result<crate::models::CatalogCache, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let db = app.state::<DbState>();
        // 短鎖讀配置 → 放鎖做網路（最多 30s）→ 短鎖寫快取：全程不佔主執行緒、不扣全局鎖做網路。
        let cfg = {
            let conn = db.conn.lock().map_err(|e| e.to_string())?;
            crate::models::catalog_read_cfg(
                &conn,
                provider_id,
                api_key.as_deref(),
                base_url.as_deref(),
            )?
        };
        let ids = crate::models::catalog_fetch_http(&cfg)?;
        let conn = db.conn.lock().map_err(|e| e.to_string())?;
        crate::models::catalog_write(&conn, provider_id, ids)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn catalog_get(
    db: State<DbState>,
    provider_id: i64,
) -> Result<crate::models::CatalogCache, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    crate::models::catalog_get(&conn, provider_id).map_err(|e| e.to_string())
}

/// 逐一實測這個模型支援哪些思考強度，並寫進記憶。
///
/// **會對上游發真正的模型請求**（1 筆基準 ＋ 8 筆候選，各生成 4 個 token），
/// 所以只在使用者按按鈕時才跑。鎖的策略與 `catalog_fetch` 相同：
/// 短鎖讀設定 → 放鎖做網路（最長 9×30 秒）→ 短鎖寫記憶。
#[tauri::command]
pub async fn model_probe_reasoning(
    app: AppHandle,
    provider_id: i64,
    model_id: i64,
) -> Result<crate::models::ReasoningProbeReport, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let db = app.state::<DbState>();
        let cfg = {
            let conn = db.conn.lock().map_err(|e| e.to_string())?;
            crate::models::probe_read_cfg(&conn, provider_id, model_id)?
        };
        let report = crate::models::probe_model_http(&cfg);
        {
            let conn = db.conn.lock().map_err(|e| e.to_string())?;
            crate::models::probe_write(&conn, provider_id, &report).map_err(|e| e.to_string())?;
        }
        Ok(report)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 逐一實測這個模型走哪一種協議（chat／responses／anthropic），並寫進記憶。
///
/// **會對上游發真正的請求**（最多 3 筆，找到就停）。用途是讓目錄知道
/// 「這個模型 Codex 到得了嗎」—— 實測只走 Anthropic 的模型不該被提供給
/// Codex（見 `models/protocol_probe.rs`）。
#[tauri::command]
pub async fn model_probe_protocol(
    app: AppHandle,
    provider_id: i64,
    model_id: i64,
) -> Result<crate::models::ProtocolProbeReport, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let db = app.state::<DbState>();
        let cfg = {
            let conn = db.conn.lock().map_err(|e| e.to_string())?;
            crate::models::probe_read_cfg(&conn, provider_id, model_id)?
        };
        let report = crate::models::probe_protocol_http(&cfg);
        {
            let conn = db.conn.lock().map_err(|e| e.to_string())?;
            crate::models::probe_protocol_write(&conn, provider_id, &report)?;
        }
        Ok(report)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 自動偵測這個來源的**連線佈局**（base ＋ 逐協議路徑前綴），並寫回設定。
///
/// 使用者只需要填「名稱／上游地址／API Key」——接口格式與四條前綴由這裡決定。
/// **會對上游發幾筆探測請求**（已知主機 4 筆、未知主機最多 12 筆）。
///
/// 安全性：只有「至少一個協議的路由存在」才寫入；全都不可達就**什麼都不動**，
/// 不會弄壞一個本來能用的來源。base 也只在**驗證過更權威的位址可達更多協議**
/// 之後才切換。
#[tauri::command]
pub async fn provider_detect_layout(
    app: AppHandle,
    provider_id: i64,
) -> Result<crate::models::LayoutReport, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let db = app.state::<DbState>();
        let cfg = {
            let conn = db.conn.lock().map_err(|e| e.to_string())?;
            crate::models::layout_read_cfg(&conn, provider_id)?
        };
        let report = crate::models::detect_layout_http(&cfg);
        {
            let conn = db.conn.lock().map_err(|e| e.to_string())?;
            crate::models::layout_write(&conn, provider_id, &report)?;
        }
        Ok(report)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 這個模型目前的思考強度記憶（UI 顯示「上次偵測」用）。
#[tauri::command]
pub fn model_reasoning_facts(
    db: State<DbState>,
    provider_id: i64,
    model: String,
) -> Result<Vec<crate::models::ReasoningFact>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    Ok(crate::models::facts(&conn, provider_id, &model))
}

#[tauri::command]
pub fn models_list(
    db: State<DbState>,
    provider_id: i64,
) -> Result<Vec<crate::models::UsableModel>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    crate::models::models_list(&conn, provider_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn model_counts(db: State<DbState>) -> Result<Vec<(i64, i64)>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    crate::models::model_counts(&conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn model_add(
    db: State<DbState>,
    input: crate::models::UsableModelInput,
) -> Result<i64, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    crate::models::model_add(&conn, &input)
}

#[tauri::command]
pub fn model_update(
    db: State<DbState>,
    id: i64,
    input: crate::models::UsableModelInput,
) -> Result<(), String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    crate::models::model_update(&conn, id, &input)
}

#[tauri::command]
pub fn model_delete(db: State<DbState>, id: i64) -> Result<(), String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    crate::models::model_delete(&conn, id)
}

// ---------------------------------------------------------------- 價格識別 ---

/// 用已加入來源的模型抽取定價（Key 只在後端使用，不寫 request_logs）。
#[tauri::command]
pub async fn price_extract(
    app: AppHandle,
    provider_id: i64,
    model: String,
    url: Option<String>,
    pasted_text: Option<String>,
    image_base64: Option<String>,
    target_hint: Option<String>,
) -> Result<crate::price_extract::PriceExtractResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let db = app.state::<DbState>();
        // 短鎖讀憑證 → 放鎖做代抓/LLM 抽取（最多 90s+15s）：不持全局鎖做網路。
        let ctx = {
            let conn = db.conn.lock().map_err(|e| e.to_string())?;
            crate::price_extract::extract_read_ctx(&conn, provider_id, &model)?
        };
        crate::price_extract::extract_run(
            &ctx,
            url.as_deref(),
            pasted_text.as_deref(),
            image_base64.as_deref(),
            target_hint.as_deref(),
        )
    })
    .await
    .map_err(|e| e.to_string())?
}
