//! Claude Desktop profile and _meta.json pure-JSON logic.
//!
//! 抽出來的理由：這些函式不碰檔案系統也不碰資料庫，可以逐條寫測試；
//! 而 mod.rs 那邊的 apply/restore 要讀路徑、寫檔、查 DB，只能靠實機驗證。
//! 兩者混在一起會讓「測得動的部分」被「測不動的部分」拖住。

use serde_json::{json, Map, Value};

use super::{PROFILE_ID, PROFILE_NAME};

// ------------------------------------------------------- 純函式（可測）---

/// 產生 Desktop 的 profile JSON。
///
/// `api_key` 是本地 Key 的明文；`models` 是要列進 `inferenceModels` 的別名。
pub(crate) fn profile_json(base_url: &str, api_key: &str, models: &[&str]) -> Value {
    json!({
        "coworkEgressAllowedHosts": ["*"],
        "disableDeploymentModeChooser": true,
        "inferenceGatewayApiKey": api_key,
        "inferenceGatewayAuthScheme": "bearer",
        "inferenceGatewayBaseUrl": base_url,
        "inferenceProvider": "gateway",
        "inferenceModels": models,
    })
}

/// 把我們的條目寫進 `_meta.json` 並把 `appliedId` 指向我們。
///
/// **保留其他條目**（CC Switch 的那份要留著，還原時要指回去）。
pub(crate) fn meta_with_us(meta: &Value) -> Value {
    let mut obj = match meta {
        Value::Object(m) => m.clone(),
        _ => Map::new(),
    };
    let mut entries: Vec<Value> = obj
        .get("entries")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    entries.retain(|e| e.get("id").and_then(|v| v.as_str()) != Some(PROFILE_ID));
    entries.push(json!({ "id": PROFILE_ID, "name": PROFILE_NAME }));
    obj.insert("entries".into(), Value::Array(entries));
    obj.insert("appliedId".into(), json!(PROFILE_ID));
    Value::Object(obj)
}

/// 把我們從 `_meta.json` 移除；若 `appliedId` 是我們，指回剩下的第一條。
///
/// 沒有其他條目時把 `appliedId` 整個移除（Desktop 會當作沒有生效的 profile）。
pub(crate) fn meta_without_us(meta: &Value) -> Value {
    let mut obj = match meta {
        Value::Object(m) => m.clone(),
        _ => Map::new(),
    };
    let mut entries: Vec<Value> = obj
        .get("entries")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    entries.retain(|e| e.get("id").and_then(|v| v.as_str()) != Some(PROFILE_ID));
    let was_ours = obj.get("appliedId").and_then(|v| v.as_str()) == Some(PROFILE_ID);
    if was_ours {
        match entries.first().and_then(|e| e.get("id")).and_then(|v| v.as_str()) {
            Some(other) => obj.insert("appliedId".into(), json!(other)),
            None => obj.remove("appliedId"),
        };
    }
    if entries.is_empty() {
        obj.remove("entries");
    } else {
        obj.insert("entries".into(), Value::Array(entries));
    }
    Value::Object(obj)
}

/// `_meta.json` 的 `appliedId` 是不是我們。
pub(crate) fn applied_is_us(meta: &Value) -> bool {
    meta.get("appliedId").and_then(|v| v.as_str()) == Some(PROFILE_ID)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claude_desktop::ALIASES;

    #[test]
    fn profile_shape_matches_desktop_expectations() {
        let v = profile_json("http://127.0.0.1:15722", "sk-local-x", &ALIASES);
        assert_eq!(v["inferenceProvider"], "gateway");
        assert_eq!(v["inferenceGatewayAuthScheme"], "bearer");
        assert_eq!(v["inferenceGatewayBaseUrl"], "http://127.0.0.1:15722");
        assert_eq!(v["inferenceGatewayApiKey"], "sk-local-x");
        assert_eq!(v["disableDeploymentModeChooser"], true);
        assert_eq!(v["coworkEgressAllowedHosts"], json!(["*"]));
        assert_eq!(v["inferenceModels"].as_array().unwrap().len(), 4);
    }

    /// 別名必須全部落在 fail-all validator 的白名單裡。
    #[test]
    fn every_alias_passes_the_fail_all_validator() {
        for a in ALIASES {
            let ok = ["claude-sonnet-", "claude-opus-", "claude-haiku-", "claude-fable-"]
                .iter()
                .any(|p| a.strip_prefix(p).is_some_and(|rest| !rest.is_empty()));
            assert!(ok, "{a} 不在 Desktop 的白名單裡，會讓整組 profile 被拒收");
        }
    }

    #[test]
    fn meta_keeps_other_profiles_and_points_at_us() {
        let other = json!({
            "appliedId": "00000000-0000-4000-8000-000000157210",
            "entries": [{ "id": "00000000-0000-4000-8000-000000157210", "name": "CC Switch" }]
        });
        let next = meta_with_us(&other);
        assert_eq!(next["appliedId"], PROFILE_ID);
        let entries = next["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 2, "別人的條目必須保留");
        assert!(entries.iter().any(|e| e["name"] == "CC Switch"));
        assert!(entries.iter().any(|e| e["id"] == PROFILE_ID));
    }

    /// 重複套用不該產生重複條目。
    #[test]
    fn meta_is_idempotent() {
        let once = meta_with_us(&json!({}));
        let twice = meta_with_us(&once);
        assert_eq!(once, twice);
        assert_eq!(twice["entries"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn restore_points_back_to_the_other_profile() {
        let other = json!({
            "appliedId": "00000000-0000-4000-8000-000000157210",
            "entries": [{ "id": "00000000-0000-4000-8000-000000157210", "name": "CC Switch" }]
        });
        let ours = meta_with_us(&other);
        let back = meta_without_us(&ours);
        assert_eq!(
            back["appliedId"], "00000000-0000-4000-8000-000000157210",
            "還原必須指回 CC Switch"
        );
        let entries = back["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["name"], "CC Switch");
        assert!(!applied_is_us(&back));
    }

    /// 只有我們一份時，還原後不該留 appliedId（否則 Desktop 會指向不存在的檔）。
    #[test]
    fn restore_without_others_clears_applied_id() {
        let ours = meta_with_us(&json!({}));
        let back = meta_without_us(&ours);
        assert!(back.get("appliedId").is_none());
        assert!(back.get("entries").is_none());
    }

    /// 還原時如果 appliedId 不是我們，不要亂改（別人的狀態不關我們的事）。
    #[test]
    fn restore_leaves_foreign_applied_id_alone() {
        let state = json!({
            "appliedId": "someone-else",
            "entries": [
                { "id": "someone-else", "name": "Other" },
                { "id": PROFILE_ID, "name": PROFILE_NAME }
            ]
        });
        let back = meta_without_us(&state);
        assert_eq!(back["appliedId"], "someone-else");
        assert_eq!(back["entries"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn applied_is_us_only_for_our_id() {
        assert!(applied_is_us(&json!({ "appliedId": PROFILE_ID })));
        assert!(!applied_is_us(&json!({ "appliedId": "x" })));
        assert!(!applied_is_us(&json!({})));
    }
}
