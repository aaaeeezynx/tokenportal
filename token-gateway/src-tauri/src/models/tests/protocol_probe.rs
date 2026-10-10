//! 協議探測與「Codex 到不了」的模型。
//!
//! 2026-10-10 的實例：opencode-go 的 `claude-haiku-5-5` 在 `/models` 清單裡，
//! 但只吃 Anthropic Messages（`/messages`）。目錄照樣把它提供給 Codex，
//! 使用者選了只會拿到「此模型不支援本協議」，而原因完全看不出來。

use super::*;

/// 起一個只認某幾種協議的假上游。
fn fake_by_path<F>(handler: F) -> u16
where
    // `Clone` 是 axum handler 的必要條件
    F: Fn(String, serde_json::Value) -> (u16, String) + Send + Sync + Clone + 'static,
{
    let app = axum::Router::new().fallback(
        move |uri: axum::http::Uri, axum::Json(body): axum::Json<serde_json::Value>| {
            let handler = handler.clone();
            async move {
                let (status, text) = handler(uri.path().to_string(), body);
                (
                    axum::http::StatusCode::from_u16(status).unwrap(),
                    text,
                )
            }
        },
    );
    let (tx, rx) = std::sync::mpsc::channel::<u16>();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async move {
            let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            tx.send(l.local_addr().unwrap().port()).unwrap();
            axum::serve(l, app).await.unwrap();
        });
    });
    rx.recv().unwrap()
}

fn cfg(port: u16, format: &str) -> ProbeCfg {
    ProbeCfg {
        base: format!("http://127.0.0.1:{port}"),
        key: "k".into(),
        scheme: "bearer".into(),
        actual_model: "claude-haiku-5-5".into(),
        format: format.into(),
    }
}

/// ★ 迴歸（2026-10-10 由真機測試抓到）：**暫時性失敗不是「協議不支援」**。
///
/// 當時 `deepseek-v4.1-flash` 的 chat／responses 剛好回 502（上游暫時故障），
/// 舊版把任何非 2xx 都當成拒絕，於是判定它「只走 Anthropic」——
/// 而目錄排除是**破壞性**的，會把一個好好的模型從 Codex 下架。
#[test]
fn transient_failure_is_inconclusive_not_rejection() {
    let port = fake_by_path(|path, _| {
        if path.ends_with("/messages") {
            (200, r#"{"type":"message"}"#.into())
        } else {
            (502, "bad gateway".into()) // 暫時性，不是拒絕
        }
    });
    let rep = probe_protocol_http(&cfg(port, "mixed"));
    assert_eq!(rep.protocol.as_deref(), Some("anthropic"));
    assert!(!rep.definitive, "有其他協議暫時失敗時不能斷言：{rep:?}");
    assert!(rep.summary.contains("無法斷言"), "{}", rep.summary);
    assert_eq!(
        rep.attempts
            .iter()
            .map(|a| a.outcome.as_str())
            .collect::<Vec<_>>(),
        vec!["inconclusive", "inconclusive", "ok"]
    );

    // ★ 核心：**不寫記憶** —— 否則目錄會把這個模型下架
    let dir = tempfile::tempdir().unwrap();
    let conn = crate::db::open_and_ensure(&dir.path().join("t.db")).unwrap();
    let pid = seed_provider(&conn, "t");
    probe_protocol_write(&conn, pid, &rep).unwrap();
    assert!(
        probed_protocol(&conn, pid, "claude-haiku-5-5").is_none(),
        "暫時性失敗不該被記成「這個模型只走 Anthropic」"
    );
    assert!(!is_unreachable_for_openai_clients(
        &conn,
        pid,
        "claude-haiku-5-5"
    ));
}

/// 限流（429）與連線失敗同樣算 inconclusive。
#[test]
fn rate_limit_and_transport_failure_are_inconclusive() {
    let port = fake_by_path(|path, _| {
        if path.ends_with("/messages") {
            (200, r#"{"type":"message"}"#.into())
        } else {
            (429, "slow down".into())
        }
    });
    let rep = probe_protocol_http(&cfg(port, "mixed"));
    assert!(!rep.definitive, "{rep:?}");

    // 連不上（埠 9 是 discard）→ status 0 → inconclusive
    let rep2 = probe_protocol_http(&cfg(9, "mixed"));
    assert_eq!(rep2.protocol, None);
    assert!(!rep2.definitive);
    assert!(
        rep2.attempts.iter().all(|a| a.outcome == "inconclusive"),
        "{rep2:?}"
    );
}

/// 明確的 4xx 拒絕 ＋ 一個成功 → 結論確定，可以據此排除。
#[test]
fn explicit_rejection_yields_a_definitive_conclusion() {
    let port = fake_by_path(|path, _| {
        if path.ends_with("/messages") {
            (200, r#"{"type":"message"}"#.into())
        } else {
            (400, r#"{"error":{"type":"ModelProtocolUnsupported"}}"#.into())
        }
    });
    let rep = probe_protocol_http(&cfg(port, "mixed"));
    assert_eq!(rep.protocol.as_deref(), Some("anthropic"));
    assert!(rep.definitive, "{rep:?}");

    let dir = tempfile::tempdir().unwrap();
    let conn = crate::db::open_and_ensure(&dir.path().join("d.db")).unwrap();
    let pid = seed_provider(&conn, "d");
    probe_protocol_write(&conn, pid, &rep).unwrap();
    assert_eq!(
        probed_protocol(&conn, pid, "claude-haiku-5-5").as_deref(),
        Some("anthropic")
    );
    assert!(is_unreachable_for_openai_clients(
        &conn,
        pid,
        "claude-haiku-5-5"
    ));
}

/// 只認 Anthropic 的模型 → 判定為 anthropic，而且**不再往下試**。
#[test]
fn protocol_probe_finds_anthropic_only_model() {
    let port = fake_by_path(|path, _| {
        if path.ends_with("/messages") {
            (200, r#"{"type":"message"}"#.into())
        } else {
            (
                400,
                r#"{"error":{"type":"ModelProtocolUnsupported"}}"#.into(),
            )
        }
    });
    let rep = probe_protocol_http(&cfg(port, "mixed"));
    assert_eq!(rep.protocol.as_deref(), Some("anthropic"));
    // chat 與 responses 都被試過並失敗，anthropic 一通就停
    assert_eq!(rep.requests, 3, "{rep:?}");
    assert_eq!(
        rep.attempts.iter().map(|a| a.protocol.as_str()).collect::<Vec<_>>(),
        vec!["chat", "responses", "anthropic"]
    );
    assert!(rep.summary.contains("Claude Code"), "{}", rep.summary);
}

/// 走 chat 的模型 → 第一筆就中，不浪費請求。
#[test]
fn protocol_probe_stops_at_first_success() {
    let port = fake_by_path(|path, _| {
        if path.ends_with("/chat/completions") {
            (200, r#"{"ok":true}"#.into())
        } else {
            (400, r#"{"error":{"type":"ModelProtocolUnsupported"}}"#.into())
        }
    });
    let rep = probe_protocol_http(&cfg(port, "mixed"));
    assert_eq!(rep.protocol.as_deref(), Some("chat"));
    assert_eq!(rep.requests, 1, "找到就該停：{rep:?}");
}

/// 三種都不通 → 不寫記憶（別把故障記成事實）。
#[test]
fn protocol_probe_records_nothing_when_all_fail() {
    let port = fake_by_path(|_, _| (503, "upstream down".into()));
    let rep = probe_protocol_http(&cfg(port, "mixed"));
    assert_eq!(rep.protocol, None);
    // 全部 5xx 是**暫時性失敗**，不是「三種都不支援」—— 文案要講對
    assert!(!rep.definitive, "{rep:?}");
    assert!(rep.summary.contains("暫時性失敗"), "{}", rep.summary);

    let dir = tempfile::tempdir().unwrap();
    let conn = crate::db::open_and_ensure(&dir.path().join("pp.db")).unwrap();
    let pid = seed_provider(&conn, "pp");
    probe_protocol_write(&conn, pid, &rep).unwrap();
    assert!(probed_protocol(&conn, pid, "claude-haiku-5-5").is_none());
}

/// 寫入後：`is_unreachable_for_openai_clients` 要成立，而且**目錄會排除它**。
#[test]
fn anthropic_only_model_is_excluded_from_the_codex_catalog() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir_all(home.join(".codex")).unwrap();
    std::fs::write(
        home.join(".codex").join("cc-switch-model-catalog.json"),
        r#"{"models": [{"slug": "tpl", "display_name": "tpl", "description": "tpl",
            "context_window": 1, "max_context_window": 1, "default_reasoning_level": "medium",
            "supported_reasoning_levels": [{"effort": "medium", "description": "d"}]}]}"#,
    )
    .unwrap();
    let data = dir.path().join("data");
    let conn = crate::db::open_and_ensure(&dir.path().join("cat.db")).unwrap();
    let pid = seed_provider(&conn, "cat");
    seed_model(&conn, pid, "claude-haiku-5-5", None);
    seed_model(&conn, pid, "deepseek-v4.1-flash", None);

    // 只有 claude 被實測成 anthropic-only
    crate::trace::remember_protocol(
        &conn,
        pid,
        "claude-haiku-5-5",
        PROBED_PROTOCOL_KEY,
        "anthropic",
    )
    .unwrap();
    crate::trace::remember_protocol(
        &conn,
        pid,
        "deepseek-v4.1-flash",
        PROBED_PROTOCOL_KEY,
        "chat",
    )
    .unwrap();

    assert!(is_unreachable_for_openai_clients(&conn, pid, "claude-haiku-5-5"));
    assert!(!is_unreachable_for_openai_clients(&conn, pid, "deepseek-v4.1-flash"));

    let dest = codex_catalog_generate(&conn, &home, &data, pid, false).unwrap();
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&dest).unwrap()).unwrap();
    let slugs: Vec<&str> = v["models"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["slug"].as_str().unwrap())
        .collect();
    assert_eq!(
        slugs,
        vec!["deepseek-v4.1-flash"],
        "只走 Anthropic 的模型不該提供給 Codex"
    );
}

/// 哨兵鍵不會干擾換手用的逐宣告記憶。
#[test]
fn probe_key_does_not_shadow_declared_protocol_memory() {
    let dir = tempfile::tempdir().unwrap();
    let conn = crate::db::open_and_ensure(&dir.path().join("k.db")).unwrap();
    let pid = seed_provider(&conn, "k");
    crate::trace::remember_protocol(&conn, pid, "m", PROBED_PROTOCOL_KEY, "anthropic").unwrap();
    crate::trace::remember_protocol(&conn, pid, "m", "mixed", "openai-chat").unwrap();
    // 換手讀的是宣告鍵，不受哨兵影響
    assert_eq!(
        crate::trace::load_protocol(&conn, pid, "m", "mixed").as_deref(),
        Some("openai-chat")
    );
    assert_eq!(probed_protocol(&conn, pid, "m").as_deref(), Some("anthropic"));
}

/// 候選全滅時的訊息要講出真正的原因，而不是「請換模型或換來源」。
#[test]
fn mismatch_message_explains_anthropic_only_models() {
    let dir = tempfile::tempdir().unwrap();
    let conn = crate::db::open_and_ensure(&dir.path().join("m.db")).unwrap();
    let pid = seed_provider(&conn, "msg");
    crate::trace::remember_protocol(&conn, pid, "claude-haiku-5-5", PROBED_PROTOCOL_KEY, "anthropic")
        .unwrap();

    let msg = protocol_mismatch_message(Some(&conn), pid, "claude-haiku-5-5", 2);
    assert!(msg.contains("Anthropic"), "{msg}");
    assert!(msg.contains("Claude Code"), "要給出可行的下一步：{msg}");
    assert!(!msg.contains("請換模型或換來源"), "別再給那句沒用的建議：{msg}");

    // 沒測過 → 引導去按偵測，而不是空泛地叫人換模型
    let msg2 = protocol_mismatch_message(Some(&conn), pid, "unknown-model", 2);
    assert!(msg2.contains("偵測"), "{msg2}");
    // 資料庫不可用也要給得出話
    let msg3 = protocol_mismatch_message(None, pid, "x", 1);
    assert!(!msg3.is_empty());
}
