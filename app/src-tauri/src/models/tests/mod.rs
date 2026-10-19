//! `models` 的單元測試。
//!
//! 原為單一 `tests.rs`；2026-10-08 因超過專案 400 行自律上限而拆成目錄：
//! - 本檔：目錄解析與可用模型 CRUD 的基本測試
//! - `catalog`：Codex 目錄生成與模板取得（含兩條真機 `#[ignore]`）
//! - `reasoning`：思考強度記憶（實測／學習）與翻譯器允許清單
//! - `probe`：思考強度探測（對假上游）

use super::*;

mod catalog;
mod layout;
mod live_protocol;
mod probe;
mod protocol_memory;
mod protocol_probe;
mod reasoning;

// ------------------------------------------------------------ 共用 helper ---
// 子模組以 `use super::*;` 取用（與 `db/tests`、`tools/tests` 同一套做法）。

/// 插一個來源，回傳 id。
fn seed_provider(conn: &rusqlite::Connection, name: &str) -> i64 {
    conn.execute(
        "INSERT INTO providers (name, app_type, api_format, base_url, api_key,
         auth_scheme, models_json, priority, enabled, created_at, updated_at)
         VALUES (?1,'universal','openai-chat','http://x','','bearer','[]',0,1,0,0)",
        [name],
    )
    .unwrap();
    conn.query_row("SELECT id FROM providers WHERE name=?1", [name], |r| {
        r.get(0)
    })
    .unwrap()
}

/// 插一個可用模型；`reasoning` 給 `None` 代表留空（未宣告）。
fn seed_model(conn: &rusqlite::Connection, pid: i64, actual: &str, reasoning: Option<&str>) {
    conn.execute(
        "INSERT INTO provider_models (provider_id, display_name, actual_model, reasoning, ord, enabled)
         VALUES (?1, ?2, ?2, ?3, 0, 1)",
        rusqlite::params![pid, actual, reasoning.unwrap_or("")],
    )
    .unwrap();
}


    #[test]
    fn parse_both_shapes() {
        let openai: serde_json::Value = serde_json::from_str(
            r#"{"object":"list","data":[{"id":"a"},{"id":"b"},{"id":"a"},{"id":""}]}"#,
        )
        .unwrap();
        assert_eq!(
            parse_catalog(&openai, "openai-chat").unwrap(),
            vec!["a".to_string(), "b".to_string()]
        );
        let gem: serde_json::Value = serde_json::from_str(
            r#"{"models":[{"name":"models/gemini-3.6-flash"},{"name":"x"}]}"#,
        )
        .unwrap();
        assert_eq!(
            parse_catalog(&gem, "gemini").unwrap(),
            vec!["gemini-3.6-flash".to_string(), "x".to_string()]
        );
        assert!(parse_catalog(&openai, "openai-chat").is_ok());
        let bad: serde_json::Value = serde_json::from_str(r#"{"foo":[]}"#).unwrap();
        assert!(parse_catalog(&bad, "openai-chat").is_err());
    }

    #[test]
    fn empty_key_fails_before_network() {
        let dir = tempfile::tempdir().unwrap();
        let conn = crate::db::open_and_ensure(&dir.path().join("t.db")).unwrap();
        let pid: i64 = conn
            .query_row("SELECT id FROM providers WHERE name='flatkey'", [], |r| {
                r.get(0)
            })
            .unwrap();
        // 種子渠道 key 為空：應直接提示填寫，不發請求
        let err = catalog_fetch(&conn, pid, None, None).unwrap_err();
        assert!(
            err.contains("API Key"),
            "應提示填寫 Key，實際：{err}"
        );
    }

    #[test]
    fn resolve_actual_prefers_display() {
        let dir = tempfile::tempdir().unwrap();
        let conn = crate::db::open_and_ensure(&dir.path().join("t.db")).unwrap();
        let pid: i64 = conn
            .query_row("SELECT id FROM providers WHERE name='flatkey'", [], |r| {
                r.get(0)
            })
            .unwrap();
        model_add(
            &conn,
            &UsableModelInput {
                provider_id: pid,
                display_name: "claude-sonnet-4-6".into(),
                actual_model: "deepseek-v4-flash".into(),
                context_window: None,
                reasoning: "unset".into(),
                protocol: String::new(),
                ord: 0,
                enabled: true,
            },
        )
        .unwrap();
        assert_eq!(
            resolve_actual(&conn, pid, "claude-sonnet-4-6"),
            "deepseek-v4-flash"
        );
        // actual 本身也命中
        assert_eq!(
            resolve_actual(&conn, pid, "deepseek-v4-flash"),
            "deepseek-v4-flash"
        );
        // 無映射原樣透傳
        assert_eq!(resolve_actual(&conn, pid, "gpt-9"), "gpt-9");
        assert_eq!(resolve_actual(&conn, pid, "  "), "");
    }

    #[test]
    fn crud_validation() {
        let dir = tempfile::tempdir().unwrap();
        let conn = crate::db::open_and_ensure(&dir.path().join("t.db")).unwrap();
        let pid: i64 = conn
            .query_row("SELECT id FROM providers WHERE name='flatkey'", [], |r| {
                r.get(0)
            })
            .unwrap();
        // 種子 models_json 已遷移為行
        let rows = models_list(&conn, pid).unwrap();
        assert!(rows.iter().any(|r| r.actual_model == "deepseek-v4-flash"));
        let base = UsableModelInput {
            provider_id: pid,
            display_name: "flash".into(),
            actual_model: "deepseek-v4-flash".into(),
            context_window: Some(128000),
            reasoning: "high".into(),
            protocol: String::new(),
            ord: 0,
            enabled: true,
        };
        let id = model_add(&conn, &base).unwrap();
        // 顯示名重複被拒
        assert!(model_add(&conn, &base).is_err());
        let mut bad = base.clone();
        bad.reasoning = "turbo".into();
        assert!(model_add(&conn, &bad).is_err());
        bad = base.clone();
        bad.context_window = Some(-1);
        assert!(model_add(&conn, &bad).is_err());
        // 逗號分隔多選推理等級
        let mut multi = base.clone();
        multi.display_name = "multi-reason".into();
        multi.reasoning = "low,medium,high".into();
        let multi_id = model_add(&conn, &multi).unwrap();
        let saved = models_list(&conn, pid)
            .unwrap()
            .into_iter()
            .find(|m| m.id == multi_id)
            .unwrap();
        assert_eq!(saved.reasoning, "low,medium,high");
        // Max 高思考等級可選
        let mut hi = base.clone();
        hi.display_name = "hi-reason".into();
        hi.reasoning = "max".into();
        let hi_id = model_add(&conn, &hi).unwrap();
        model_delete(&conn, hi_id).unwrap();
        // 含無效值的多選被拒
        let mut bad_multi = base.clone();
        bad_multi.display_name = "bad-multi".into();
        bad_multi.reasoning = "low,turbo".into();
        assert!(model_add(&conn, &bad_multi).is_err());
        model_delete(&conn, multi_id).unwrap();
        // 更新 + 刪除
        let mut upd = base.clone();
        upd.display_name = "flash2".into();
        model_update(&conn, id, &upd).unwrap();
        assert!(model_update(&conn, 999999, &upd).is_err());
        model_delete(&conn, id).unwrap();
        assert!(model_delete(&conn, id).is_err());
    }

