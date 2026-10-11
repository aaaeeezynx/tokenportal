//! 端到端：**單一來源同時服務多種協議**（2026-10-10）。
//!
//! 這是訂閱制上游的實際形狀：opencode-go 一個 base_url、一把金鑰，
//! `claude-haiku-5-5` 只吃 Anthropic Messages、其餘模型吃 OpenAI chat。
//! 舊版只能用來源級的 `api_format` 描述，使用者被迫把同一個訂閱拆成兩個來源
//! —— 金鑰重複，App 的用量／成本／健康度統計也被切碎。
//!
//! 這裡驗證的是完整鏈路：`auto` ＋ 逐模型協議 ＋ 逐協議鑑權 ＋ 換手。

use super::*;

/// 假上游：三種端點都活著，各自記下收到的請求（路徑 + 是否帶 x-api-key）。
#[allow(clippy::type_complexity)]
fn fake_three_protocols(
    messages_ok: bool,
) -> (u16, std::sync::Arc<std::sync::Mutex<Vec<(String, bool)>>>) {
    let seen: std::sync::Arc<std::sync::Mutex<Vec<(String, bool)>>> =
        std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen_srv = seen.clone();
    let app = axum::Router::new().fallback(
        move |uri: axum::http::Uri, req: axum::http::Request<axum::body::Body>| {
            let seen = seen_srv.clone();
            async move {
                let path = uri.path().to_string();
                let has_key = req.headers().contains_key("x-api-key");
                seen.lock().unwrap().push((path.clone(), has_key));
                if path.ends_with("/messages") {
                    if !messages_ok {
                        return (
                            axum::http::StatusCode::BAD_REQUEST,
                            r#"{"type":"error","error":{"type":"ModelProtocolUnsupported","message":"Model does not support this protocol."}}"#
                                .to_string(),
                        );
                    }
                    // 缺 x-api-key 就當 401 —— 照 opencode-go 的真實行為
                    if !has_key {
                        return (
                            axum::http::StatusCode::UNAUTHORIZED,
                            r#"{"type":"error","error":{"type":"AuthError","message":"Missing API key."}}"#
                                .to_string(),
                        );
                    }
                    return (
                        axum::http::StatusCode::OK,
                        r#"{"id":"msg_1","type":"message","role":"assistant","model":"m",
                            "content":[{"type":"text","text":"hi"}],
                            "stop_reason":"end_turn",
                            "usage":{"input_tokens":1,"output_tokens":1}}"#
                            .to_string(),
                    );
                }
                // chat / responses 走 OpenAI 相容回包
                (
                    axum::http::StatusCode::OK,
                    r#"{"id":"c1","object":"chat.completion","model":"m",
                        "choices":[{"index":0,"finish_reason":"stop",
                                    "message":{"role":"assistant","content":"hi"}}],
                        "usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#
                        .to_string(),
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
    (rx.recv().unwrap(), seen)
}

/// 建立一個 `auto` 來源，兩個模型各自標好協議。回傳 (埠, 本地 Key, seen, db, dir)。
#[allow(clippy::type_complexity)]
async fn setup(
    messages_ok: bool,
) -> (
    u16,
    String,
    std::sync::Arc<std::sync::Mutex<Vec<(String, bool)>>>,
    tempfile::TempDir,
) {
    let (fake_port, seen) = fake_three_protocols(messages_ok);
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("auto.db");
    let secret = {
        let conn = crate::db::open_and_ensure(&db_path).unwrap();
        let pid = seed_provider(&conn, "sub-src");
        // 來源是 auto（新預設）；協議逐模型決定
        conn.execute(
            "UPDATE providers SET api_format='auto', base_url=?1 WHERE name='sub-src'",
            [format!("http://127.0.0.1:{fake_port}")],
        )
        .unwrap();
        seed_model(&conn, pid, "claude-ish", None);
        seed_model(&conn, pid, "chatty", None);
        seed_model(&conn, pid, "unknown-ish", None);
        conn.execute(
            "UPDATE provider_models SET protocol='anthropic' WHERE actual_model='claude-ish'",
            [],
        )
        .unwrap();
        conn.execute(
            "UPDATE provider_models SET protocol='openai-chat' WHERE actual_model='chatty'",
            [],
        )
        .unwrap();
        // unknown-ish 刻意留空：模擬「還沒偵測過」的模型
        seed_key(&conn, pid, "auto")
    };
    let gw_l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let gw_port = gw_l.local_addr().unwrap().port();
    tokio::spawn(serve(db_path, gw_l));
    (gw_port, secret, seen, dir)
}

fn anthropic_body(model: &str) -> serde_json::Value {
    serde_json::json!({
        "model": model,
        "max_tokens": 16,
        "messages": [{"role": "user", "content": "hi"}]
    })
}

/// ★ 核心：Anthropic 入站 ＋ 模型協議 anthropic → **直通 `/messages`**，且帶 x-api-key。
///
/// 舊行為：`mixed` 來源的 Anthropic 請求一律被翻成 chat 送到 `/chat/completions`，
/// 所以 Anthropic-only 的模型（如 opencode-go 的 claude-haiku-5-5）必定失敗。
#[tokio::test]
async fn auto_source_passes_anthropic_model_through_to_messages() {
    let (gw, secret, seen, _dir) = setup(true).await;
    let r = Client::new()
        .post(format!("http://127.0.0.1:{gw}/v1/messages"))
        .bearer_auth(&secret)
        .json(&anthropic_body("claude-ish"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200, "Anthropic 模型應該直通成功");

    let s = seen.lock().unwrap().clone();
    assert_eq!(s.len(), 1, "不該有失敗往返：{s:?}");
    assert!(s[0].0.ends_with("/messages"), "應打到 /messages：{s:?}");
    assert!(s[0].1, "Anthropic 目標必須帶 x-api-key：{s:?}");
}

/// 協議**已知**是 chat → 直接走 `/chat/completions`，**不浪費**一次往返。
///
/// 這是逐模型協議的價值之一：沒有它就得每次先撞一次錯的端點。
#[tokio::test]
async fn known_chat_protocol_goes_straight_to_chat() {
    let (gw, secret, seen, _dir) = setup(false).await;
    let r = Client::new()
        .post(format!("http://127.0.0.1:{gw}/v1/messages"))
        .bearer_auth(&secret)
        .json(&anthropic_body("chatty"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);

    let s = seen.lock().unwrap().clone();
    assert_eq!(s.len(), 1, "協議已知就不該有失敗往返：{s:?}");
    assert!(s[0].0.contains("/chat/completions"), "{s:?}");
}

/// 協議**未知**（還沒偵測）→ 先試最自然的 `/messages`，被拒後自動換到
/// `/chat/completions`。使用者只多付一次往返，之後由 `learn_format` 記住。
#[tokio::test]
async fn unknown_protocol_fails_over_from_messages_to_chat() {
    let (gw, secret, seen, _dir) = setup(false).await;
    let r = Client::new()
        .post(format!("http://127.0.0.1:{gw}/v1/messages"))
        .bearer_auth(&secret)
        .json(&anthropic_body("unknown-ish"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200, "換到 chat 之後應該成功");

    let s = seen.lock().unwrap().clone();
    assert!(s.len() >= 2, "應先試 messages 再換 chat：{s:?}");
    assert!(s[0].0.ends_with("/messages"), "{s:?}");
    assert!(
        s.last().unwrap().0.contains("/chat/completions"),
        "最後應打到 /chat/completions：{s:?}"
    );
}
