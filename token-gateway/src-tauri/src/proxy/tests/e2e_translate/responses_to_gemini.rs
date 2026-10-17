//! 端到端：**Codex（Responses）→ Gemini 上游**（2026-10-11 的反向翻譯）。
//!
//! 請求側兩跳：`responses_to_openai` → `chat_to_gemini`；
//! 回應側兩跳：`gemini_chunk_to_chat_chunks` → `chat_chunk_to_responses`。
//!
//! 這裡也驗證 Gemini 最特別的兩件事：**模型名在路徑裡**（`build_upstream_target`
//! 必須用翻譯器給的路徑覆寫）與 **`x-goog-api-key` 鑑權**。

use super::super::*;

/// 假 Gemini 上游：只認正確的路徑與鑑權，並斷言收到的形狀。
async fn gemini_upstream(stream: bool) -> u16 {
    use axum::response::IntoResponse;
    let app = axum::Router::new().fallback(
        move |uri: axum::http::Uri,
              headers: axum::http::HeaderMap,
              axum::Json(body): axum::Json<serde_json::Value>| {
            async move {
                let path = uri.path().to_string();
                let query = uri.query().unwrap_or("").to_string();
                // ★ 模型名必須在路徑裡
                let want = if stream {
                    "/v1beta/models/gemini-x:streamGenerateContent"
                } else {
                    "/v1beta/models/gemini-x:generateContent"
                };
                assert_eq!(path, want, "路徑不對（模型名要在裡面）");
                if stream {
                    assert!(query.contains("alt=sse"), "串流要 alt=sse：{query}");
                }
                // ★ Gemini 的鑑權
                assert_eq!(
                    headers.get("x-goog-api-key").and_then(|v| v.to_str().ok()),
                    Some("up-key"),
                    "應帶 x-goog-api-key"
                );
                // ★ 形狀：contents/parts（不是 messages）
                assert!(body.get("contents").is_some(), "{body}");
                assert!(body.get("messages").is_none(), "不該殘留 Chat 的 messages");
                assert_eq!(body["contents"][0]["role"], "user");
                assert_eq!(body["contents"][0]["parts"][0]["text"], "hi");
                // Responses 的 instructions → Chat system → Gemini systemInstruction
                assert_eq!(body["systemInstruction"]["parts"][0]["text"], "be brief");
                // 工具換成 functionDeclarations
                assert_eq!(body["tools"][0]["functionDeclarations"][0]["name"], "get_weather");

                if !stream {
                    return axum::Json(serde_json::json!({
                        "candidates": [{"content": {"role": "model",
                            "parts": [{"text": "hello from gemini"}]},
                            "finishReason": "STOP"}],
                        "usageMetadata": {"promptTokenCount": 11, "candidatesTokenCount": 5}
                    }))
                    .into_response();
                }
                // Gemini 的 SSE：data 是完整的 GenerateContentResponse，沒有 [DONE]
                let s = format!(
                    "data: {}\n\ndata: {}\n\ndata: {}\n\n",
                    serde_json::json!({"candidates": [{"content": {"role": "model",
                        "parts": [{"text": "hello "}]}}]}),
                    serde_json::json!({"candidates": [{"content": {"role": "model",
                        "parts": [{"text": "from gemini"}]}}]}),
                    serde_json::json!({"candidates": [{"content": {"parts": []},
                        "finishReason": "STOP"}],
                        "usageMetadata": {"promptTokenCount": 11, "candidatesTokenCount": 5}}),
                );
                (
                    [(axum::http::header::CONTENT_TYPE, "text/event-stream")],
                    s,
                )
                    .into_response()
            }
        },
    );
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    port
}

/// 建好 Gemini 來源（宣告 `gemini` ＋ `goog-key`）與本地 Key。
async fn setup(stream: bool) -> (u16, String, tempfile::TempDir) {
    let up = gemini_upstream(stream).await;
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("g2.db");
    let secret = {
        let conn = crate::db::open_and_ensure(&db_path).unwrap();
        let pid = seed_provider(&conn, "gem-src");
        conn.execute(
            "UPDATE providers SET api_format='gemini', auth_scheme='goog-key',
             base_url=?1, api_key='up-key' WHERE name='gem-src'",
            [format!("http://127.0.0.1:{up}/v1beta")],
        )
        .unwrap();
        seed_model(&conn, pid, "gemini-x", None);
        seed_key(&conn, pid, "g2")
    };
    let gw_l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let gw_port = gw_l.local_addr().unwrap().port();
    tokio::spawn(serve(db_path, gw_l));
    (gw_port, secret, dir)
}

fn codex_body(stream: bool) -> serde_json::Value {
    serde_json::json!({
        "model": "gemini-x",
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

/// 非串流：Codex 的 Responses 請求 → Gemini → 拿回 Responses 形狀。
#[tokio::test]
async fn codex_request_is_translated_to_gemini_and_back() {
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
    assert_eq!(body["model"], "gemini-x", "回顯請求模型");
    let text = body["output"]
        .as_array()
        .and_then(|a| a.iter().find(|o| o["type"] == "message"))
        .map(|m| m["content"][0]["text"].as_str().unwrap_or("").to_string())
        .unwrap_or_default();
    assert_eq!(text, "hello from gemini", "{body}");
    assert_eq!(body["usage"]["input_tokens"], 11);
    assert_eq!(body["usage"]["output_tokens"], 5);
}

/// 串流（Codex 的實際模式）：Gemini SSE → Chat chunk → Responses 事件。
#[tokio::test]
async fn codex_streaming_gets_responses_events_from_a_gemini_upstream() {
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
    assert!(text.contains("response.created"), "{text}");
    assert!(text.contains("response.output_text.delta"), "{text}");
    assert!(text.contains("response.completed"), "{text}");
    assert!(text.contains("hello "), "{text}");
    assert!(text.contains("from gemini"), "{text}");
    // 不該把 Gemini 的形狀漏給客戶端
    assert!(!text.contains("candidates"), "不該漏出 Gemini 的 candidates：{text}");
    assert!(!text.contains("finishReason"), "不該漏出 Gemini 的 finishReason：{text}");
}
