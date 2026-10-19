//! 連線佈局自動偵測（`models/layout.rs`）。
//!
//! 判定關鍵是「**非 404 就算路由存在**」—— 即使上游回 `ModelProtocolUnsupported`，
//! 也證明路徑是對的。所以不必知道模型支援哪些協議就能判斷佈局。

use super::*;

/// 起一個假上游；`handler(path) -> (status, body)`。
fn fake<F>(handler: F) -> u16
where
    F: Fn(String) -> (u16, String) + Send + Sync + Clone + 'static,
{
    let app = axum::Router::new().fallback(
        move |uri: axum::http::Uri| {
            let handler = handler.clone();
            async move {
                let (status, text) = handler(uri.path().to_string());
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

fn cfg(port: u16) -> LayoutCfg {
    LayoutCfg {
        base: format!("http://127.0.0.1:{port}"),
        key: "k".into(),
        scheme: "bearer".into(),
        model: "m".into(),
    }
}

/// ★ 核心：**前綴式佈局**（opencode 那種）要被認出來，而且四種協議的路徑都對。
#[test]
fn detects_the_prefixed_layout() {
    let port = fake(|path| {
        let ok = path.starts_with("/openai/v1/")
            || path.starts_with("/anthropic/v1/")
            || path.starts_with("/google/v1beta/");
        if ok {
            (200, "{}".into())
        } else {
            (404, "<html>not found</html>".into())
        }
    });
    let rep = detect_layout_http(&cfg(port));
    assert!(!rep.base_changed, "未知主機不該改 base");
    assert_eq!(rep.reachable.len(), 4, "{:?}", rep.routes);
    // 前綴要存成 JSON
    let p = crate::models::ProtocolPrefixes::parse(&rep.prefixes);
    assert_eq!(p.for_protocol("openai-chat"), "/openai/v1");
    assert_eq!(p.for_protocol("openai-responses"), "/openai/v1");
    assert_eq!(p.for_protocol("anthropic"), "/anthropic/v1");
    assert_eq!(p.for_protocol("gemini"), "/google/v1beta");
}

/// 標準佈局（沒有前綴）：`{base}/chat/completions` 等 —— 存的前綴要是**空的**。
#[test]
fn detects_the_plain_layout_and_stores_no_prefixes() {
    let port = fake(|path| {
        let ok = path == "/chat/completions"
            || path == "/responses"
            || path == "/messages"
            || path.starts_with("/models/");
        if ok {
            (200, "{}".into())
        } else {
            (404, "not found".into())
        }
    });
    let rep = detect_layout_http(&cfg(port));
    assert_eq!(rep.reachable.len(), 4, "{:?}", rep.routes);
    assert_eq!(rep.prefixes, "", "沒有前綴就不該存 JSON");
}

/// ★ 判定關鍵：**400 也算路由存在**（模型不收 ≠ 路徑不對）。
#[test]
fn a_non_404_error_still_proves_the_route_exists() {
    let port = fake(|path| {
        if path.contains("chat/completions") {
            (
                400,
                r#"{"error":{"type":"ModelProtocolUnsupported"}}"#.into(),
            )
        } else {
            (404, "not found".into())
        }
    });
    let rep = detect_layout_http(&cfg(port));
    assert_eq!(rep.reachable, vec!["openai-chat".to_string()], "{:?}", rep.routes);
    assert_eq!(rep.routes[0].status, 400);
}

/// 全都 404 → **不寫任何東西**（絕不弄壞一個本來能用的來源）。
#[test]
fn nothing_reachable_writes_nothing() {
    let port = fake(|_| (404, "not found".into()));
    let rep = detect_layout_http(&cfg(port));
    assert!(rep.reachable.is_empty());
    assert!(rep.message.contains("都不存在"), "{}", rep.message);

    let dir = tempfile::tempdir().unwrap();
    let conn = crate::db::open_and_ensure(&dir.path().join("l.db")).unwrap();
    let pid = seed_provider(&conn, "lay");
    conn.execute(
        "UPDATE providers SET api_format='openai-chat', protocol_prefixes='{\"anthropic\":\"/x\"}'
         WHERE id=?1",
        [pid],
    )
    .unwrap();
    layout_write(&conn, pid, &rep).unwrap();
    // 設定必須原封不動
    let (fmt, pre): (String, String) = conn
        .query_row(
            "SELECT api_format, protocol_prefixes FROM providers WHERE id=?1",
            [pid],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(fmt, "openai-chat", "失敗時不該把接口格式改成 auto");
    assert_eq!(pre, "{\"anthropic\":\"/x\"}", "失敗時不該動前綴");
}

/// 成功時寫入：base／前綴／`api_format` 都設成偵測結果。
#[test]
fn a_successful_detection_is_persisted() {
    let rep = LayoutReport {
        base_url: "https://example.test/go".into(),
        base_changed: true,
        prefixes: r#"{"openai-chat":"/openai/v1"}"#.into(),
        routes: vec![],
        reachable: vec!["openai-chat".into()],
        message: String::new(),
    };
    let dir = tempfile::tempdir().unwrap();
    let conn = crate::db::open_and_ensure(&dir.path().join("w.db")).unwrap();
    let pid = seed_provider(&conn, "w");
    layout_write(&conn, pid, &rep).unwrap();
    let (base, fmt, pre): (String, String, String) = conn
        .query_row(
            "SELECT base_url, api_format, protocol_prefixes FROM providers WHERE id=?1",
            [pid],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(base, "https://example.test/go");
    assert_eq!(fmt, "auto", "偵測成功後接口格式一律 auto");
    assert_eq!(pre, r#"{"openai-chat":"/openai/v1"}"#);
}

/// 已知主機表：opencode.ai 要認得，而且它宣告的四種前綴要完整。
#[test]
fn known_host_table_covers_opencode() {
    let k = known_for("https://opencode.ai/inference/go").expect("opencode 應在已知清單");
    assert_eq!(
        k.canonical_base,
        Some("https://opencode.ai/inference/go"),
        "權威 base 要跟官方文件一致"
    );
    for p in ["openai-chat", "openai-responses", "anthropic", "gemini"] {
        assert!(
            k.prefixes.iter().any(|(name, _)| *name == p),
            "缺 {p} 的前綴"
        );
    }
    // 不認得的主機 → None（走實測）
    assert!(known_for("https://api.example.test/v1").is_none());
}

/// 端點字尾的形狀（Gemini 的模型名在路徑裡）。
#[test]
fn suffix_shapes_are_correct() {
    assert_eq!(suffix_for("openai-chat", "m"), "/chat/completions");
    assert_eq!(suffix_for("openai-responses", "m"), "/responses");
    assert_eq!(suffix_for("anthropic", "m"), "/messages");
    assert_eq!(
        suffix_for("gemini", "gemini-x"),
        "/models/gemini-x:generateContent"
    );
    // 使用者填 models/ 前綴不該重複
    assert_eq!(
        suffix_for("gemini", "models/gemini-x"),
        "/models/gemini-x:generateContent"
    );
}
