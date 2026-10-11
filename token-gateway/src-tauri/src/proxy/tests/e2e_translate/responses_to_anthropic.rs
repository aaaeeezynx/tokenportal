//! 端到端：**Codex（Responses）→ Anthropic 上游**（2026-10-10 的反向翻譯）。
//!
//! 這是使用者真正要的那條路：讓 Codex 用得了只走 Anthropic Messages 的模型
//! （例如 opencode-go 的 `claude-haiku-5-5`）。
//!
//! 請求側是**兩跳**：`responses_to_openai` → `chat_to_anthropic`；
//! 回應側也是兩跳：`anthropic_event_to_chat_chunks` → `chat_chunk_to_responses`。
//! 這裡驗證整條鏈路的形狀。

use super::super::*;

/// 起一個 Anthropic 形狀的假上游；`stream` 決定回非串流還是 SSE。
async fn anthropic_upstream(stream: bool) -> u16 {
    use axum::response::IntoResponse;
    let app = axum::Router::new().route(
        "/messages",
        axum::routing::post(move |body: axum::Json<serde_json::Value>| async move {
            // ★ 上游收到的必須是**Anthropic 形狀**
            assert_eq!(body["model"], "claude-ish", "{body:?}");
            assert!(
                body.get("input").is_none(),
                "不該殘留 Responses 的 input 欄位：{body:?}"
            );
            assert_eq!(body["messages"][0]["role"], "user");
            assert_eq!(body["messages"][0]["content"][0]["type"], "text");
            assert_eq!(body["messages"][0]["content"][0]["text"], "hi");
            // system 要拉到頂層（Responses 的 instructions → Chat system → Anthropic system）
            assert_eq!(body["system"], "be brief");
            assert!(body["max_tokens"].as_i64().unwrap_or(0) > 0, "必填：{body:?}");
            // 工具要換成 Anthropic 的 input_schema
            assert_eq!(body["tools"][0]["name"], "get_weather");
            assert!(body["tools"][0].get("input_schema").is_some(), "{body:?}");

            if !stream {
                return axum::Json(serde_json::json!({
                    "id": "msg_1", "type": "message", "role": "assistant",
                    "model": "claude-ish",
                    "content": [{"type": "text", "text": "hello from claude"}],
                    "stop_reason": "end_turn",
                    "usage": {"input_tokens": 11, "output_tokens": 5}
                }))
                .into_response();
            }
            // Anthropic SSE：data 自帶 type
            let ev = |v: serde_json::Value| format!("event: x\ndata: {v}\n\n");
            let s = format!(
                "{}{}{}{}{}{}",
                ev(serde_json::json!({"type":"message_start","message":{"id":"msg_1",
                    "model":"claude-ish","usage":{"input_tokens":11,"output_tokens":0}}})),
                ev(serde_json::json!({"type":"content_block_start","index":0,
                    "content_block":{"type":"text","text":""}})),
                ev(serde_json::json!({"type":"content_block_delta","index":0,
                    "delta":{"type":"text_delta","text":"hello "}})),
                ev(serde_json::json!({"type":"content_block_delta","index":0,
                    "delta":{"type":"text_delta","text":"from claude"}})),
                ev(serde_json::json!({"type":"content_block_stop","index":0})),
                ev(serde_json::json!({"type":"message_delta",
                    "delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":5}})),
            ) + &ev(serde_json::json!({"type":"message_stop"}));
            (
                [(axum::http::header::CONTENT_TYPE, "text/event-stream")],
                s,
            )
                .into_response()
        }),
    );
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    port
}

/// 建好來源（宣告 anthropic）與本地 Key。
async fn setup(stream: bool) -> (u16, String, tempfile::TempDir) {
    let up = anthropic_upstream(stream).await;
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("r2a.db");
    let secret = {
        let conn = crate::db::open_and_ensure(&db_path).unwrap();
        let pid = seed_provider(&conn, "claude-src");
        conn.execute(
            "UPDATE providers SET api_format='anthropic', base_url=?1, auth_scheme='anthropic'
             WHERE name='claude-src'",
            [format!("http://127.0.0.1:{up}")],
        )
        .unwrap();
        seed_model(&conn, pid, "claude-ish", None);
        seed_key(&conn, pid, "r2a")
    };
    let gw_l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let gw_port = gw_l.local_addr().unwrap().port();
    tokio::spawn(serve(db_path, gw_l));
    (gw_port, secret, dir)
}

fn codex_body(stream: bool) -> serde_json::Value {
    serde_json::json!({
        "model": "claude-ish",
        "instructions": "be brief",
        "input": [{"type": "message", "role": "user",
                   "content": [{"type": "input_text", "text": "hi"}]}],
        "tools": [{"type": "function", "name": "get_weather",
                   "description": "w", "parameters": {"type": "object"},
                   "strict": false}],
        "max_output_tokens": 64,
        "stream": stream
    })
}

/// 非串流：Codex 的 Responses 請求 → Anthropic → 拿回 Responses 形狀。
#[tokio::test]
async fn responses_request_is_translated_to_anthropic_and_back() {
    let (gw, secret, _dir) = setup(false).await;
    let r = Client::new()
        .post(format!("http://127.0.0.1:{gw}/v1/responses"))
        .bearer_auth(&secret)
        .json(&codex_body(false))
        .send()
        .await
        .unwrap();
    let status = r.status();
    let body: serde_json::Value = r.json().await.unwrap();
    assert_eq!(status, 200, "{body}");
    // ★ 客戶端拿到的是 **Responses** 形狀
    assert_eq!(body["object"], "response");
    assert_eq!(body["model"], "claude-ish", "回顯請求模型");
    let text = body["output"]
        .as_array()
        .and_then(|a| a.iter().find(|o| o["type"] == "message"))
        .map(|m| m["content"][0]["text"].as_str().unwrap_or("").to_string())
        .unwrap_or_default();
    assert_eq!(text, "hello from claude", "{body}");
    assert_eq!(body["usage"]["input_tokens"], 11);
    assert_eq!(body["usage"]["output_tokens"], 5);
}

/// 串流（Codex 的實際模式）：Anthropic SSE → Chat chunk → Responses 事件。
#[tokio::test]
async fn codex_streaming_gets_responses_events_from_an_anthropic_upstream() {
    let (gw, secret, _dir) = setup(true).await;
    let r = Client::new()
        .post(format!("http://127.0.0.1:{gw}/v1/responses"))
        .bearer_auth(&secret)
        .json(&codex_body(true))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let text = r.text().await.unwrap();
    // Responses 的事件形狀
    assert!(text.contains("response.created"), "{text}");
    assert!(text.contains("response.output_text.delta"), "{text}");
    assert!(text.contains("response.completed"), "{text}");
    // 兩個 delta 都要到（"hello " 與 "from claude"）
    assert!(text.contains("hello "), "{text}");
    assert!(text.contains("from claude"), "{text}");
    // 不該把 Anthropic 的事件名漏出去
    assert!(!text.contains("content_block_delta"), "不該漏出上游事件名：{text}");
    assert!(!text.contains("message_stop"), "不該漏出上游事件名：{text}");
}
