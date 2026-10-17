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
        prefixes: String::new(),
    }
}

/// 起一個「只有 Gemini 路徑與 `x-goog-api-key` 才通」的假上游。
///
/// 回傳 `(port, 觀察到的請求)`；每筆記下 `(路徑, 有沒有 x-goog-api-key)`。
/// 測試據此斷言**鑑權與形狀都對** —— 只斷言「探測成功」看不出是哪一項錯了。
#[allow(clippy::type_complexity)]
fn fake_gemini_upstream() -> (u16, std::sync::Arc<std::sync::Mutex<Vec<(String, bool)>>>) {
    let seen: std::sync::Arc<std::sync::Mutex<Vec<(String, bool)>>> =
        std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen_srv = seen.clone();
    let app = axum::Router::new().fallback(
        move |uri: axum::http::Uri,
              headers: axum::http::HeaderMap,
              axum::Json(body): axum::Json<serde_json::Value>| {
            let seen = seen_srv.clone();
            async move {
                let path = uri.path().to_string();
                let has_key = headers.contains_key("x-goog-api-key");
                seen.lock().unwrap().push((path.clone(), has_key));
                // 只有 Gemini 的路徑與鑑權才通
                if !path.contains(":generateContent") {
                    return (axum::http::StatusCode::NOT_FOUND, "not found".to_string());
                }
                if !has_key {
                    return (
                        axum::http::StatusCode::UNAUTHORIZED,
                        "missing x-goog-api-key".to_string(),
                    );
                }
                // 形狀：Gemini 用 contents/parts，不是 chat 的 messages
                if body.get("contents").is_none() {
                    return (
                        axum::http::StatusCode::BAD_REQUEST,
                        format!("expected contents/parts, got {body}"),
                    );
                }
                (axum::http::StatusCode::OK, r#"{"candidates":[]}"#.to_string())
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
    (rx.recv().unwrap(), seen)
}




/// ★ 探測器要認得 Gemini：**模型名在路徑裡**、內容是 `contents/parts`、
/// 鑑權是 `x-goog-api-key`。
///
/// 2026-10-11 之前探測只試三種協議，Gemini 模型會被報成「三種都不通」——
/// 那是誤導，它其實只是第四種。
#[test]
fn protocol_probe_detects_gemini() {
    let (port, seen) = fake_gemini_upstream();
    let mut c = cfg(port, "gemini");
    c.actual_model = "gemini-2.5-flash".into();
    let rep = probe_protocol_http(&c);

    assert_eq!(rep.protocol.as_deref(), Some("gemini"), "{rep:?}");
    assert!(rep.definitive, "{rep:?}");
    assert_eq!(rep.requests, 1, "來源宣告 gemini → 第一筆就中");

    let s = seen.lock().unwrap().clone();
    assert_eq!(s.len(), 1, "{s:?}");
    assert!(
        s[0].0.contains("/models/gemini-2.5-flash:generateContent"),
        "模型名要在路徑裡：{s:?}"
    );
    assert!(s[0].1, "Gemini 必須帶 x-goog-api-key：{s:?}");
    assert!(rep.summary.contains("Gemini"), "{}", rep.summary);
}

/// ★ 迴歸（2026-10-11 真機誤判）：Gemini 的**路徑前綴各家不同**，探測不能只試
/// 一種形狀。
///
/// 實例：opencode-go 的文件端點是
/// `/inference/go/google/v1beta/models/{m}:generateContent` —— 那個
/// `/google/v1beta` 不是「base + 字尾」拼得出來的。寫死標準形會讓可用的 Gemini
/// 模型被誤報成「四種協議都不通」（`gemini-4-argon-contributor` 就是這樣）。
#[test]
fn gemini_probe_falls_through_to_the_protocol_prefixed_path() {
    let port = fake_by_path(|path, body| {
        // 只認 `/google/v1beta/...`（模擬 opencode-go 的真實形狀）
        if path.contains("/google/v1beta/models/") && path.contains(":generateContent") {
            assert!(body.get("contents").is_some(), "{body}");
            (200, r#"{"candidates":[]}"#.into())
        } else {
            (404, "<html>not found</html>".into())
        }
    });
    let mut c = cfg(port, "auto");
    c.actual_model = "gemini-4-argon-contributor".into();
    let rep = probe_protocol_http(&c);
    assert_eq!(rep.protocol.as_deref(), Some("gemini"), "{rep:?}");
    // chat/responses/anthropic 各 1 筆，gemini 試了 2 個形狀（標準形 404 → 前綴形 200）
    assert_eq!(rep.requests, 5, "{rep:?}");
    assert!(rep.summary.contains("Gemini"), "{}", rep.summary);
}

/// 標準形就通時**不該**多打其他形狀（省請求）。
#[test]
fn gemini_probe_stops_at_the_first_working_shape() {
    let port = fake_by_path(|path, _| {
        if path.ends_with(":generateContent") {
            (200, r#"{"candidates":[]}"#.into())
        } else {
            (404, "not found".into())
        }
    });
    let mut c = cfg(port, "gemini");
    c.actual_model = "gemini-x".into();
    let rep = probe_protocol_http(&c);
    assert_eq!(rep.protocol.as_deref(), Some("gemini"), "{rep:?}");
    assert_eq!(rep.requests, 1, "宣告 gemini → 第一筆就中，不該再試其他形狀");
}

/// 路由存在但模型不收（400）→ **就是這個協議的結論**，不必再試其他形狀
/// （但還是會繼續試其餘三種協議 —— 模型也可能在 chat 上架）。
#[test]
fn gemini_probe_stops_on_a_non_404_rejection() {
    let port = fake_by_path(|path, _| {
        if path.contains(":generateContent") {
            (400, r#"{"error":{"message":"Model does not support"}}"#.into())
        } else {
            (404, "not found".into())
        }
    });
    let rep = probe_protocol_http(&cfg(port, "gemini"));
    assert_eq!(rep.protocol, None, "{rep:?}");
    // gemini 排第一（來源宣告）→ 400 就定案，**只試了 1 個路徑形狀**
    assert_eq!(rep.attempts[0].protocol, "gemini", "{rep:?}");
    assert_eq!(rep.attempts[0].status, 400, "{rep:?}");
    // 總請求數 = 4（gemini 1 筆 ＋ 其餘三種各 1 筆）；
    // 若 gemini 把 3 個形狀都試了就會是 6 —— 那才是要防的浪費。
    assert_eq!(rep.requests, 4, "{rep:?}");
}

/// `models/` 前綴不想被重複：使用者填 `models/gemini-x` 也要能通。
#[test]
fn probe_strips_redundant_models_prefix_for_gemini() {
    let (port, seen) = fake_gemini_upstream();
    let mut c = cfg(port, "gemini");
    c.actual_model = "models/gemini-2.5-flash".into();
    let rep = probe_protocol_http(&c);
    assert_eq!(rep.protocol.as_deref(), Some("gemini"), "{rep:?}");
    let s = seen.lock().unwrap().clone();
    assert!(
        s[0].0.contains("/models/gemini-2.5-flash:generateContent"),
        "不該變成 /models/models/…：{s:?}"
    );
}

/// 來源明確宣告某種協議時，探測先試它（省請求）。
#[test]
fn probe_order_tries_the_declared_protocol_first() {
    assert_eq!(probe_order("gemini")[0], "gemini");
    assert_eq!(probe_order("anthropic")[0], "anthropic");
    assert_eq!(probe_order("openai-responses")[0], "responses");
    assert_eq!(probe_order("openai-chat")[0], "chat");
    // mixed / auto 沒有單一答案 → 保持原順序
    assert_eq!(probe_order("mixed")[0], "chat");
    assert_eq!(probe_order("auto")[0], "chat");
    // 四種都要在
    let mut all = probe_order("auto");
    all.sort_unstable();
    assert_eq!(all, vec!["anthropic", "chat", "gemini", "responses"]);
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
    assert!(probed_protocol(&conn, pid, "claude-haiku-5-5").is_none());
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
    assert_eq!(
        probed_protocol(&conn, pid, "claude-haiku-5-5").as_deref(),
        Some("anthropic")
    );
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

/// ★ 迴歸（2026-10-11）：Anthropic-only 的模型**必須**被收進 Codex 目錄。
///
/// 反向翻譯器（OpenAI → Anthropic）實作之前，這裡曾經反過來 —— 目錄會把
/// 這種模型排除掉。那在當時是對的（提供了也永遠失敗），但翻譯器做好之後
/// 就變成「明明能用卻在選單裡看不到」，實例是 opencode-go 的
/// `claude-haiku-5-5`。
#[test]
fn anthropic_only_model_is_included_in_the_codex_catalog() {
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

    crate::trace::remember_protocol(
        &conn,
        pid,
        "claude-haiku-5-5",
        PROBED_PROTOCOL_KEY,
        "anthropic",
    )
    .unwrap();

    let dest = codex_catalog_generate(&conn, &home, &data, pid, false).unwrap();
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&dest).unwrap()).unwrap();
    let slugs: Vec<&str> = v["models"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["slug"].as_str().unwrap())
        .collect();
    assert!(
        slugs.contains(&"claude-haiku-5-5"),
        "Anthropic-only 的模型現在用得了，必須出現在目錄裡：{slugs:?}"
    );
    assert!(slugs.contains(&"deepseek-v4.1-flash"), "{slugs:?}");
}
