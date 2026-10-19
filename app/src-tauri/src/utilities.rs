//! 內建小工具（對標 CC Switch 的 built-in utilities）。
//!
//! 三個都是「對 Claude Code 的設定檔做**最小增量**修改」——
//! 只碰那一個鍵，其餘欄位（含使用者自己加的）原樣保留：
//!
//! | 小工具 | 檔案 | 鍵 | 開啟 | 關閉 |
//! |---|---|---|---|---|
//! | 跳過初次安裝確認 | `~/.claude.json` | `hasCompletedOnboarding` | 寫 `true` | 刪除該鍵 |
//! | 隱藏 AI 署名 | `~/.claude/settings.json` | `includeCoAuthoredBy` | 寫 `false` | 刪除該鍵 |
//! | 編輯器擴充整合 | `~/.claude/config.json` | `primaryApiKey` | 寫 `"any"` | 刪除該鍵 |
//!
//! ## 為什麼「關閉」是刪鍵而不是寫反向值
//!
//! 三個鍵的預設值各不相同（`includeCoAuthoredBy` 預設是 true、
//! `hasCompletedOnboarding` 預設不存在），寫反向值會把使用者的原始設定
//! 永久改掉。刪鍵＝回到「這個 App 沒碰過」的狀態，最小的侵入。
//!
//! ## 為什麼不用覆蓋整個檔案
//!
//! `~/.claude.json` 可能有幾百 KB 的專案歷史與其他鍵；整檔重寫等於把
//! 我們不認識的東西一起重排。所以一律「讀成 Map → 改一個鍵 → 寫回」，
//! 並走 `fsutil::atomic_write`（先寫 `.tmp` 再 rename，不會寫壞一半）。
//!
//! 出處：CC Switch v4.0.4 的 `claude_plugin.rs`／`claude_mcp.rs`
//! （`set_has_completed_onboarding`、`write_claude_config`），
//! 以及 `settings.rs` 的 `includeCoAuthoredBy: false`。行為對齊，程式碼自寫。

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{Map, Value};

use crate::fsutil;

/// 三個小工具的識別碼（前端用）。
pub const KEY_ONBOARDING: &str = "skipOnboarding";
pub const KEY_ATTRIBUTION: &str = "hideAttribution";
pub const KEY_PLUGIN: &str = "pluginIntegration";

const REL_CLAUDE_JSON: &str = ".claude.json";
const REL_SETTINGS: &str = ".claude/settings.json";
const REL_CONFIG: &str = ".claude/config.json";

fn home() -> Result<PathBuf, String> {
    dirs::home_dir().ok_or_else(|| "無法解析使用者主目錄".to_string())
}

/// 小工具狀態。
#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UtilitiesState {
    pub skip_onboarding: bool,
    pub hide_attribution: bool,
    pub plugin_integration: bool,
    /// 三個目標檔案的實際路徑與是否存在（前端顯示「會改哪個檔」）
    pub targets: Vec<UtilityTarget>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UtilityTarget {
    pub key: &'static str,
    pub path: String,
    pub exists: bool,
}

/// 讀 JSON 物件；檔案不存在回空 Map，壞掉則回錯（**不覆蓋壞檔**）。
fn read_obj(path: &Path) -> Result<Map<String, Value>, String> {
    if !path.exists() {
        return Ok(Map::new());
    }
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("讀取 {} 失敗：{e}", path.display()))?;
    if text.trim().is_empty() {
        return Ok(Map::new());
    }
    match serde_json::from_str::<Value>(&text) {
        Ok(Value::Object(m)) => Ok(m),
        Ok(_) => Err(format!("{} 的根不是物件，為避免破壞原檔已中止", path.display())),
        Err(e) => Err(format!(
            "{} 不是合法 JSON（{e}）——請先手動修好，本程式不會覆蓋它",
            path.display()
        )),
    }
}

fn write_obj(path: &Path, obj: &Map<String, Value>) -> Result<(), String> {
    let text = serde_json::to_string_pretty(obj)
        .map_err(|e| format!("序列化失敗：{e}"))?;
    fsutil::atomic_write(path, format!("{text}\n").as_bytes())
        .map_err(|e| format!("寫入 {} 失敗：{e}", path.display()))
}

/// 開啟＝插入該鍵；關閉＝刪除該鍵（回到「沒碰過」的狀態）。
fn apply(obj: &mut Map<String, Value>, key: &str, on: bool, on_value: Value) {
    if on {
        obj.insert(key.to_string(), on_value);
    } else {
        obj.remove(key);
    }
}

/// 讀單一旗標的目前狀態。
fn flag(obj: &Map<String, Value>, key: &str, want: &Value) -> bool {
    obj.get(key).is_some_and(|v| v == want)
}

/// 目標檔案路徑（開啟／關閉都指向同一個）。
fn rel_for(key: &str) -> Option<&'static str> {
    match key {
        KEY_ONBOARDING => Some(REL_CLAUDE_JSON),
        KEY_ATTRIBUTION => Some(REL_SETTINGS),
        KEY_PLUGIN => Some(REL_CONFIG),
        _ => None,
    }
}

/// 該鍵「開啟」時要寫入的值。
fn on_value(key: &str) -> Option<Value> {
    match key {
        KEY_ONBOARDING => Some(Value::Bool(true)),
        KEY_ATTRIBUTION => Some(Value::Bool(false)),
        KEY_PLUGIN => Some(Value::String("any".to_string())),
        _ => None,
    }
}

/// JSON 內的鍵名。
fn json_key(key: &str) -> Option<&'static str> {
    match key {
        KEY_ONBOARDING => Some("hasCompletedOnboarding"),
        KEY_ATTRIBUTION => Some("includeCoAuthoredBy"),
        KEY_PLUGIN => Some("primaryApiKey"),
        _ => None,
    }
}

/// 套用（或還原）一個小工具。回傳「這次有沒有真的改到盤」。
pub(crate) fn apply_at(root: &Path, key: &str, enabled: bool) -> Result<bool, String> {
    let rel = rel_for(key).ok_or_else(|| format!("未知的小工具：{key}"))?;
    let jkey = json_key(key).ok_or_else(|| format!("未知的小工具：{key}"))?;
    let want = on_value(key).ok_or_else(|| format!("未知的小工具：{key}"))?;
    let path = root.join(rel);

    let mut obj = read_obj(&path)?;
    let before = flag(&obj, jkey, &want);
    if before == enabled {
        // 已經是目標狀態：不寫檔（避免無謂地動使用者的檔案時間戳）
        return Ok(false);
    }
    apply(&mut obj, jkey, enabled, want);
    write_obj(&path, &obj)?;
    Ok(true)
}

/// 讀目前狀態（三個旗標 ＋ 三個路徑）。
pub(crate) fn read_state_at(root: &Path) -> UtilitiesState {
    let mut st = UtilitiesState::default();
    for key in [KEY_ONBOARDING, KEY_ATTRIBUTION, KEY_PLUGIN] {
        let rel = rel_for(key).unwrap_or("");
        let jkey = json_key(key).unwrap_or("");
        let want = on_value(key).unwrap_or(Value::Null);
        let path = root.join(rel);
        let on = read_obj(&path)
            .map(|o| flag(&o, jkey, &want))
            .unwrap_or(false);
        match key {
            KEY_ONBOARDING => st.skip_onboarding = on,
            KEY_ATTRIBUTION => st.hide_attribution = on,
            KEY_PLUGIN => st.plugin_integration = on,
            _ => {}
        }
        st.targets.push(UtilityTarget {
            key: match key {
                KEY_ONBOARDING => KEY_ONBOARDING,
                KEY_ATTRIBUTION => KEY_ATTRIBUTION,
                _ => KEY_PLUGIN,
            },
            path: path.display().to_string(),
            exists: path.exists(),
        });
    }
    st
}

// ---------------------------------------------------------------- 命令 ---

#[tauri::command]
pub fn utilities_state() -> Result<UtilitiesState, String> {
    Ok(read_state_at(&home()?))
}

/// 開啟／關閉一個小工具，回傳套用後的最新狀態。
#[tauri::command]
pub fn utilities_set(key: String, enabled: bool) -> Result<UtilitiesState, String> {
    let root = home()?;
    apply_at(&root, &key, enabled)?;
    Ok(read_state_at(&root))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> tempfile::TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    fn read(path: &Path) -> Value {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    /// 開啟＝寫入該鍵；關閉＝把鍵刪掉（不是寫 false）。
    #[test]
    fn onboarding_roundtrip_creates_and_removes_the_key() {
        let d = tmp();
        let p = d.path().join(REL_CLAUDE_JSON);

        assert!(apply_at(d.path(), KEY_ONBOARDING, true).unwrap());
        assert_eq!(read(&p)["hasCompletedOnboarding"], Value::Bool(true));
        assert!(read_state_at(d.path()).skip_onboarding);

        assert!(apply_at(d.path(), KEY_ONBOARDING, false).unwrap());
        assert!(
            read(&p).get("hasCompletedOnboarding").is_none(),
            "關閉必須是刪鍵，不是寫 false"
        );
        assert!(!read_state_at(d.path()).skip_onboarding);
    }

    /// 只碰目標鍵：使用者自己加的東西一個都不能掉。
    #[test]
    fn other_keys_are_preserved() {
        let d = tmp();
        let p = d.path().join(REL_CLAUDE_JSON);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(
            &p,
            r#"{"projects":{"/a":{"x":1}},"numStartups":42,"userID":"u-1"}"#,
        )
        .unwrap();

        apply_at(d.path(), KEY_ONBOARDING, true).unwrap();
        let v = read(&p);
        assert_eq!(v["hasCompletedOnboarding"], Value::Bool(true));
        assert_eq!(v["numStartups"], 42);
        assert_eq!(v["userID"], "u-1");
        assert_eq!(v["projects"]["/a"]["x"], 1);
    }

    /// 三個鍵寫在三個不同的檔案裡，不能互相污染。
    #[test]
    fn each_utility_targets_its_own_file() {
        let d = tmp();
        apply_at(d.path(), KEY_ATTRIBUTION, true).unwrap();
        apply_at(d.path(), KEY_PLUGIN, true).unwrap();

        let s = read(&d.path().join(REL_SETTINGS));
        let c = read(&d.path().join(REL_CONFIG));
        assert_eq!(s["includeCoAuthoredBy"], Value::Bool(false));
        assert_eq!(c["primaryApiKey"], Value::String("any".into()));
        assert!(s.get("primaryApiKey").is_none(), "不能寫錯檔");
        assert!(c.get("includeCoAuthoredBy").is_none(), "不能寫錯檔");
        // 沒碰過的那個不該被建立
        assert!(!d.path().join(REL_CLAUDE_JSON).exists());
    }

    /// 重複設定同一個狀態時不寫檔（時間戳不變）。
    #[test]
    fn idempotent_does_not_touch_the_file() {
        let d = tmp();
        assert!(apply_at(d.path(), KEY_PLUGIN, true).unwrap());
        let p = d.path().join(REL_CONFIG);
        let before = std::fs::metadata(&p).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1200));
        assert!(
            !apply_at(d.path(), KEY_PLUGIN, true).unwrap(),
            "第二次應該回 false（沒改）"
        );
        let after = std::fs::metadata(&p).unwrap().modified().unwrap();
        assert_eq!(before, after, "已經是同一個狀態就不該重寫檔");
    }

    /// 壞掉的 JSON 不可以被覆蓋。
    #[test]
    fn invalid_json_is_refused_not_clobbered() {
        let d = tmp();
        let p = d.path().join(REL_SETTINGS);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, "{ not json ").unwrap();

        let err = apply_at(d.path(), KEY_ATTRIBUTION, true).unwrap_err();
        assert!(err.contains("不是合法 JSON"), "錯誤訊息要講清楚：{err}");
        assert_eq!(
            std::fs::read_to_string(&p).unwrap(),
            "{ not json ",
            "原檔必須原封不動"
        );
    }

    /// 根不是物件時也不覆蓋（例如檔案內容是 `[]`）。
    #[test]
    fn non_object_root_is_refused() {
        let d = tmp();
        let p = d.path().join(REL_CONFIG);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, "[]").unwrap();
        let err = apply_at(d.path(), KEY_PLUGIN, true).unwrap_err();
        assert!(err.contains("根不是物件"), "{err}");
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "[]");
    }

    #[test]
    fn unknown_key_is_rejected() {
        let d = tmp();
        assert!(apply_at(d.path(), "nope", true).is_err());
        assert!(rel_for("nope").is_none());
        assert!(json_key("nope").is_none());
        assert!(on_value("nope").is_none());
    }

    /// 狀態回報要含三個路徑，前端才知道會改哪個檔。
    #[test]
    fn state_reports_targets() {
        let d = tmp();
        let st = read_state_at(d.path());
        assert_eq!(st.targets.len(), 3);
        assert!(st.targets.iter().any(|t| t.path.ends_with(".claude.json")));
        assert!(st.targets.iter().any(|t| t.path.ends_with("settings.json")));
        assert!(st.targets.iter().any(|t| t.path.ends_with("config.json")));
        assert!(st.targets.iter().all(|t| !t.exists), "空目錄下都還不存在");
        assert!(!st.skip_onboarding && !st.hide_attribution && !st.plugin_integration);
    }
}
