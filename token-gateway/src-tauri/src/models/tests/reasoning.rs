//! 思考強度記憶（實測／學習）與翻譯器允許清單。
//!
//! 這一組釘住的是 2026-10-08 那個 400 的根治：目錄與翻譯層都不能再讓
//! 「上游明確不收」的強度過去。

use super::*;

/// 記憶的往返：寫進去、讀得出來、同鍵覆蓋。
#[test]
fn reasoning_memory_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let conn = crate::db::open_and_ensure(&dir.path().join("r.db")).unwrap();
    let pid = seed_provider(&conn, "rmem");

    assert!(facts(&conn, pid, "m").is_empty(), "一開始沒有紀錄");
    record(&conn, pid, "m", "medium", true, "probe", 1000).unwrap();
    record(&conn, pid, "m", "xhigh", false, "probe", 1000).unwrap();
    record(&conn, pid, "m", "max", false, "learned", 2000).unwrap();

    assert_eq!(known_supported(&conn, pid, "m"), vec!["medium".to_string()]);
    let mut uns = known_unsupported(&conn, pid, "m");
    uns.sort();
    assert_eq!(uns, vec!["max".to_string(), "xhigh".to_string()]);

    // 同鍵覆蓋（probe 之後學到相反的答案，以最後寫入為準）
    record(&conn, pid, "m", "xhigh", true, "probe", 3000).unwrap();
    assert_eq!(
        known_supported(&conn, pid, "m"),
        vec!["medium".to_string(), "xhigh".to_string()]
    );

    // 別的來源看不到
    assert!(facts(&conn, pid + 1, "m").is_empty());
    // 空模型名不寫入
    record(&conn, pid, "  ", "low", true, "probe", 4000).unwrap();
    assert!(!known_supported(&conn, pid, "").contains(&"low".to_string()));
}

/// `allowed_efforts` 的三段判定 ＋ 一律扣掉已知不支援。
#[test]
fn allowed_efforts_prefers_declaration_then_probe_then_default() {
    let dir = tempfile::tempdir().unwrap();
    let conn = crate::db::open_and_ensure(&dir.path().join("a.db")).unwrap();
    let pid = seed_provider(&conn, "aef");
    let m = "model-x";

    // ① 什麼都沒有 → 保守預設（**不含 xhigh／max**，那兩個實測會 400）
    let d = allowed_efforts(&conn, pid, m);
    assert_eq!(d, vec!["low", "medium", "high"]);

    // ② 有一次完整的實測 → 用支援集合（其餘候選都記成不支援，模擬真實偵測）
    for lv in CANDIDATE_LEVELS {
        record(
            &conn,
            pid,
            m,
            lv,
            matches!(lv, "low" | "medium" | "high"),
            "probe",
            1000,
        )
        .unwrap();
    }
    assert_eq!(
        allowed_efforts(&conn, pid, m),
        vec!["low".to_string(), "medium".to_string(), "high".to_string()]
    );

    // ③ 使用者宣告優先於記憶…
    seed_model(&conn, pid, m, Some("low,medium,high,xhigh,max"));
    let got = allowed_efforts(&conn, pid, m);
    assert!(
        !got.contains(&"xhigh".to_string()) && !got.contains(&"max".to_string()),
        "已知不支援的 xhigh 必須被扣掉，實得 {got:?}"
    );
    assert_eq!(got, vec!["low".to_string(), "medium".to_string(), "high".to_string()]);

    // …但宣告「全部不支援」時就什麼都不轉發（讓上游用預設，而不是撞 400）
    for lv in ["low", "medium", "high"] {
        record(&conn, pid, m, lv, false, "probe", 5000).unwrap();
    }
    assert!(allowed_efforts(&conn, pid, m).is_empty());
}

/// 目錄生成要吃記憶：宣告含 `xhigh` 也會被扣掉，Codex 就不能再選到它。
#[test]
fn catalog_drops_levels_the_upstream_rejects() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir_all(home.join(".codex")).unwrap();
    std::fs::write(
        home.join(".codex").join("cc-switch-model-catalog.json"),
        r#"{"models": [{"slug": "tpl", "display_name": "tpl", "description": "tpl",
            "context_window": 1, "max_context_window": 1, "default_reasoning_level": "medium",
            "supported_reasoning_levels": [
                {"effort": "low", "description": "d"},
                {"effort": "medium", "description": "d"},
                {"effort": "high", "description": "d"},
                {"effort": "xhigh", "description": "d"},
                {"effort": "max", "description": "d"}]}]}"#,
    )
    .unwrap();
    let data = dir.path().join("data");
    let conn = crate::db::open_and_ensure(&dir.path().join("c.db")).unwrap();
    let pid = seed_provider(&conn, "cdrop");
    // 使用者宣告了 5 個，但上游實測只收 3 個
    seed_model(&conn, pid, "kimi-test", Some("low,medium,high,xhigh,max"));
    for lv in ["low", "medium", "high"] {
        record(&conn, pid, "kimi-test", lv, true, "probe", 1000).unwrap();
    }
    for lv in ["xhigh", "max"] {
        record(&conn, pid, "kimi-test", lv, false, "probe", 1000).unwrap();
    }

    let dest = codex_catalog_generate(&conn, &home, &data, pid, false).unwrap();
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&dest).unwrap()).unwrap();
    let m = &v["models"][0];
    let lv: Vec<&str> = m["supported_reasoning_levels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["effort"].as_str().unwrap())
        .collect();
    assert_eq!(lv, vec!["low", "medium", "high"], "xhigh／max 必須被扣掉");
    // 預設檔位若被濾掉也要跟著改，否則 Codex 會選到一個不存在的預設
    assert_eq!(m["default_reasoning_level"], "low");
}

/// 空值 ＋ 有偵測紀錄 → 宣告「該模型可用的全部強度」。
#[test]
fn empty_declaration_uses_probed_supported_set() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir_all(home.join(".codex")).unwrap();
    std::fs::write(
        home.join(".codex").join("cc-switch-model-catalog.json"),
        r#"{"models": [{"slug": "tpl", "display_name": "tpl", "description": "tpl",
            "context_window": 1, "max_context_window": 1, "default_reasoning_level": "medium",
            "supported_reasoning_levels": [
                {"effort": "low", "description": "d"},
                {"effort": "medium", "description": "d"},
                {"effort": "high", "description": "d"}]}]}"#,
    )
    .unwrap();
    let data = dir.path().join("data");
    let conn = crate::db::open_and_ensure(&dir.path().join("e.db")).unwrap();
    let pid = seed_provider(&conn, "empty");
    seed_model(&conn, pid, "m2", None); // 宣告留空
    for lv in ["low", "medium", "high"] {
        record(&conn, pid, "m2", lv, true, "probe", 1000).unwrap();
    }
    record(&conn, pid, "m2", "max", false, "probe", 1000).unwrap();

    let dest = codex_catalog_generate(&conn, &home, &data, pid, false).unwrap();
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&dest).unwrap()).unwrap();
    let lv: Vec<&str> = v["models"][0]["supported_reasoning_levels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["effort"].as_str().unwrap())
        .collect();
    assert_eq!(lv, vec!["low", "medium", "high"], "應用實測支援集合");
}
