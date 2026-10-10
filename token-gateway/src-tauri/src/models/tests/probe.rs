//! 思考強度探測（對假上游）。
//!
//! 這裡釘住的重點是**判定規則**，尤其是「基準失敗時整輪中止」——
//! 沒有那道基準，一個金鑰過期或模型名打錯的模型會被誤判成
//! 「所有強度都不支援」，然後把使用者的設定清掉。

use super::*;

/// 起一個假上游：`/chat/completions` 的行為由傳入的 closure 決定。
///
/// 回傳 `(port, 收到的請求數)`。用 `Arc<Mutex<..>>` 記請求數，
/// 因為探測是阻塞式（reqwest::blocking），伺服器必須跑在別的执行緒上。
fn fake_upstream<F>(handler: F) -> (u16, std::sync::Arc<std::sync::Mutex<Vec<String>>>)
where
    F: Fn(Option<String>) -> (u16, String) + Send + Sync + Clone + 'static,
{
    let seen: std::sync::Arc<std::sync::Mutex<Vec<String>>> =
        std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen_srv = seen.clone();
    let app = axum::Router::new().fallback(
        move |axum::Json(body): axum::Json<serde_json::Value>| {
            let seen = seen_srv.clone();
            async move {
                let lv = body
                    .get("reasoning_effort")
                    .or_else(|| body.get("reasoning").and_then(|r| r.get("effort")))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                seen.lock().unwrap().push(lv.clone().unwrap_or_else(|| "__none__".into()));
                let (status, text) = handler(lv);
                (
                    axum::http::StatusCode::from_u16(status).unwrap(),
                    text,
                )
            }
        },
    );
    // 探測端是阻塞式的，所以伺服器跑在自己的執行緒 + 自己的 runtime 上
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
    (rx.recv().unwrap(), seen)
}

fn cfg_for(port: u16) -> ProbeCfg {
    ProbeCfg {
        base: format!("http://127.0.0.1:{port}"),
        key: "k".into(),
        scheme: "bearer".into(),
        actual_model: "mimo-test".into(),
        format: "openai-chat".into(),
    }
}

/// 基準通過 → 逐一分類候選；400 算不支援、200 算支援、429 算「無法判定」。
#[test]
fn probe_classifies_candidates_and_skips_inconclusive() {
    let (port, seen) = fake_upstream(|lv| match lv.as_deref() {
        None => (200, r#"{"ok":true}"#.into()),          // 基準
        Some("low") | Some("medium") => (200, r#"{"ok":true}"#.into()),
        Some("xhigh") => (400, r#"{"error":"Invalid request parameters"}"#.into()),
        Some("max") => (429, r#"{"error":"rate limited"}"#.into()),
        Some(_) => (400, r#"{"error":"Invalid request parameters"}"#.into()),
    });
    let rep = probe_model_http(&cfg_for(port));

    assert!(rep.baseline_ok, "{rep:?}");
    assert!(rep.aborted.is_none());
    assert_eq!(rep.requests, CANDIDATE_LEVELS.len() + 1);
    assert!(rep.supported.contains(&"low".to_string()));
    assert!(rep.supported.contains(&"medium".to_string()));
    assert!(rep.unsupported.contains(&"xhigh".to_string()));
    // 429 不是強度的問題 → 既不支援也不列入 unsupported，而是 inconclusive
    assert!(!rep.unsupported.contains(&"max".to_string()));
    assert!(rep.inconclusive.iter().any(|o| o.level == "max"));
    assert!(!rep.supported.contains(&"max".to_string()));
    // 基準確實先送（第一筆是 __none__）
    assert_eq!(seen.lock().unwrap().first().unwrap(), "__none__");
}

/// 基準 401 → 整輪中止，**一個候選都不測**、記憶也不寫。
#[test]
fn probe_aborts_on_bad_credentials_without_testing_candidates() {
    let (port, seen) = fake_upstream(|_| (401, r#"{"error":"bad key"}"#.into()));
    let rep = probe_model_http(&cfg_for(port));

    assert!(!rep.baseline_ok);
    assert!(rep.aborted.as_deref().unwrap().contains("金鑰或權限"));
    assert_eq!(rep.requests, 1, "只該送出基準那一筆");
    assert!(rep.supported.is_empty() && rep.unsupported.is_empty());
    assert_eq!(seen.lock().unwrap().len(), 1);
}

/// 基準 404（模型名打錯）→ 中止，而且訊息要指出是路徑／模型名。
#[test]
fn probe_aborts_on_missing_endpoint() {
    let (port, _) = fake_upstream(|_| (404, "404 page not found".into()));
    let rep = probe_model_http(&cfg_for(port));
    assert!(rep.aborted.as_deref().unwrap().contains("端點或模型名"));
}

/// 基準 5xx → 中止，且歸類為上游故障。
#[test]
fn probe_aborts_on_upstream_failure() {
    let (port, _) = fake_upstream(|_| (503, "upstream down".into()));
    let rep = probe_model_http(&cfg_for(port));
    assert!(rep.aborted.as_deref().unwrap().contains("上游故障"));
}

/// 真機驗證：對**真實的** opencode-go 兩個 MiMo 跑一次探測。
///
/// 這是整個功能的驗收標準：2026-10-08 手動實測得到的結論是
/// 「兩個模型都只吃 low／medium／high，其餘五個全部 400」。
/// 探測器必須自己得出同一個答案。
///
/// `cargo test --offline --lib -- --ignored live_probe --nocapture`
#[test]
#[ignore = "live: 需要真實來源與金鑰（會對上游發 18 筆極小請求）"]
fn live_probe_two_mimo_models() {
    let db = dirs::data_dir()
        .expect("data_dir")
        .join("com.tokencounter.gateway")
        .join("app.db");
    let conn = crate::db::open_and_ensure(&db).expect("打開真實資料庫");

    let mut checked = 0;
    for name in ["mimo-v2.6-pro", "mimo-v2.6-flash"] {
        let row: Option<(i64, i64)> = conn
            .query_row(
                "SELECT m.provider_id, m.id FROM provider_models m
                 JOIN providers p ON p.id = m.provider_id
                 WHERE m.actual_model = ?1 AND p.enabled = 1 LIMIT 1",
                [name],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .ok();
        let Some((pid, mid)) = row else {
            println!("（跳過 {name}：資料庫裡找不到）");
            continue;
        };
        let cfg = match probe_read_cfg(&conn, pid, mid) {
            Ok(c) => c,
            Err(e) => {
                println!("（跳過 {name}：{e}）");
                continue;
            }
        };
        let rep = probe_model_http(&cfg);
        assert!(rep.baseline_ok, "{name} 基準失敗：{:?}", rep.aborted);
        assert_eq!(
            rep.supported,
            vec!["low".to_string(), "medium".to_string(), "high".to_string()],
            "{name} 的支援集合不符手動實測結果；unsupported={:?} inconclusive={:?}",
            rep.unsupported,
            rep.inconclusive
        );
        for lv in ["minimal", "xhigh", "max", "ultra", "persistent"] {
            assert!(
                rep.unsupported.contains(&lv.to_string()),
                "{name} 應判定 {lv} 不支援，實得 {:?}",
                rep.unsupported
            );
        }
        println!("✅ {name}：{:?}", rep.supported);
        checked += 1;
    }
    assert!(checked > 0, "沒有任何模型被驗證到（來源或金鑰不在？）");
}

/// 寫入記憶與「帶入」設定：只有明確答案才落庫，inconclusive 不寫。
#[test]
fn probe_write_records_only_definite_answers() {
    let dir = tempfile::tempdir().unwrap();
    let conn = crate::db::open_and_ensure(&dir.path().join("p.db")).unwrap();
    let pid = seed_provider(&conn, "pwrite");

    let rep = ReasoningProbeReport {
        model: "m".into(),
        baseline_ok: true,
        aborted: None,
        supported: vec!["low".into(), "medium".into()],
        unsupported: vec!["xhigh".into()],
        inconclusive: vec![LevelOutcome {
            level: "max".into(),
            supported: false,
            detail: "HTTP 429".into(),
        }],
        requests: 9,
        checked_at: 1,
    };
    probe_write(&conn, pid, &rep).unwrap();

    let sup = known_supported(&conn, pid, "m");
    assert_eq!(sup, vec!["low".to_string(), "medium".to_string()]);
    let uns = known_unsupported(&conn, pid, "m");
    assert_eq!(uns, vec!["xhigh".to_string()]);
    // 不明確的 max 不該被記成「不支援」
    assert!(!uns.contains(&"max".to_string()));
    // 基準本身也記一筆（代表「不帶 effort 是活的」）
    assert!(facts(&conn, pid, "m").iter().any(|f| f.level == LEVEL_NONE));

    // ★ 「帶入」：使用者的宣告被改寫成實測支援集合（這就是功能名稱的兌現）。
    //   只有明確按偵測才會動它；被動學習不會（見 proxy/retry.rs）。
    seed_model(&conn, pid, "m", Some("low,medium,high,xhigh"));
    probe_write(&conn, pid, &rep).unwrap();
    let declared: String = conn
        .query_row(
            "SELECT reasoning FROM provider_models WHERE provider_id=?1 AND actual_model='m'",
            [pid],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(declared, "low,medium", "宣告應被帶入為實測支援集合");

    // 基準失敗的報告什麼都不寫
    let bad = ReasoningProbeReport {
        model: "m2".into(),
        baseline_ok: false,
        aborted: Some("boom".into()),
        ..Default::default()
    };
    probe_write(&conn, pid, &bad).unwrap();
    assert!(facts(&conn, pid, "m2").is_empty());
}
