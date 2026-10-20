//! 串流（SSE）分派：依 TransKind 選一個轉送分支。
//!
//! 從 finish.rs 抽出來的（該檔原本 456 行，超過專案的 400 行上限）。
//! **分支內容一字未改**，只是換了外層 wrapper 並把共用變數改成參數。
//!
//! 每個分支都是「spawn 一個 task 邊收邊轉邊送，流結束時記帳」，
//! 差別只在 on_line 用哪個轉換器：
//!
//! | TransKind | 上游 | 客戶端 | 轉換器 |
//! |---|---|---|---|
//! | ResponsesToGemini | Gemini | Responses | gemini_line_to_responses |
//! | ChatToGemini | Gemini | Chat | gemini_line_to_chat |
//! | AnthropicToGemini | Gemini | Anthropic | gemini chunk → chat chunk → anthropic event |
//! | ResponsesToAnthropic | Anthropic | Responses | anthropic_line_to_responses |
//! | ChatToAnthropic | Anthropic | Chat | anthropic_line_to_chat |
//! | ResponsesToChat | Chat | Responses | responses_line_events |
//!
//! 注意：每個 spawn 分支都要在 async move 之外先把字串複製成 owned
//!（例如 f.req.app.to_string()）—— f 是借用，tokio::spawn 要求 'static。

use super::*;

/// 串流分派。回傳要給客戶端的 SSE 響應。
pub(super) fn dispatch_sse(
    upstream: reqwest::Response,
    builder: axum::http::response::Builder,
    f: &FinishCtx<'_>,
    status: StatusCode,
    latency_ms: i64,
    log_fallback: String,
    custom_tools: Vec<String>,
) -> Response {
    let db_path = f.req.ctx.db_path.clone();
    let provider_id = f.provider_id;
    let key_id = f.key_id;
    // 流式：邊轉邊解析，結束寫日誌（翻譯分支按 kind 轉換事件形狀）
    let (tx, rx) =
        tokio::sync::mpsc::channel::<Result<axum::body::Bytes, axum::Error>>(64);
    if f.kind == TransKind::ResponsesToGemini {
        // Gemini 上游 → Codex：Gemini chunk → Chat chunk → Responses 事件
        let appv = f.req.app.to_string();
        let echo = f.req.model_raw.to_string();
        let logm = log_fallback.clone();
        let customs = custom_tools.clone();
        tokio::spawn(async move {
            let on_line = |s: &mut ResponsesFromGeminiRelay, t: &str| {
                gemini_line_to_responses(t, s, &echo, &customs)
            };
            let on_finish = |s: &mut ResponsesFromGeminiRelay| {
                crate::translate::responses_stream_finish(
                    &mut s.tstate,
                    &echo,
                    &s.acc,
                    &customs,
                )
            };
            let st = relay_sse(
                upstream,
                &tx,
                ResponsesFromGeminiRelay::default(),
                on_line,
                on_finish,
            )
            .await;
            StreamLog::new(
                &db_path,
                &appv,
                provider_id,
                key_id,
                latency_ms,
                status.as_u16(),
            )
            .write(&logm, &st.acc);
        });
        return sse_response(builder, rx);
    }
    if f.kind == TransKind::ChatToGemini {
        // Gemini 上游 → Chat 客戶端。Gemini 沒有 [DONE]，由 on_finish 補。
        let appv = f.req.app.to_string();
        let logm = log_fallback.clone();
        tokio::spawn(async move {
            let on_line = |s: &mut ChatFromGeminiRelay, t: &str| {
                gemini_line_to_chat(t, s)
            };
            let on_finish = |_s: &mut ChatFromGeminiRelay| {
                vec!["data: [DONE]\n\n".to_string()]
            };
            let st = relay_sse(
                upstream,
                &tx,
                ChatFromGeminiRelay::default(),
                on_line,
                on_finish,
            )
            .await;
            StreamLog::new(
                &db_path,
                &appv,
                provider_id,
                key_id,
                latency_ms,
                status.as_u16(),
            )
            .write(&logm, &st.acc);
        });
        return sse_response(builder, rx);
    }
    if f.kind == TransKind::AnthropicToGemini {
        // Gemini 上游 → Anthropic 客戶端（Claude Code）：兩跳
        // Gemini chunk → Chat chunk → Anthropic 事件。
        // 兩個轉換器都是既有的，這裡只負責把它們串起來。
        let appv = f.req.app.to_string();
        let echo = f.req.model_raw.to_string();
        let logm = log_fallback.clone();
        tokio::spawn(async move {
            let on_line = |s: &mut AnthropicFromGeminiRelay, t: &str| {
                let Some(d) = t.strip_prefix("data:") else {
                    return (vec![], false);
                };
                let d = d.trim_start();
                if d.is_empty() || d == "[DONE]" {
                    return (vec![], false);
                }
                let Ok(v) = serde_json::from_str::<serde_json::Value>(d) else {
                    return (vec![], false);
                };
                // 上游錯誤 → Anthropic error 事件並收流
                if let Some(err) = v.get("error") {
                    let err = if err.is_null() { v.clone() } else { err.clone() };
                    return (vec![crate::translate::anthropic_error_line(&err)], true);
                }
                let mut out: Vec<String> = Vec::new();
                for chunk in
                    crate::translate::gemini_chunk_to_chat_chunks(&mut s.gstate, &v)
                {
                    out.extend(crate::translate::openai_chunk_to_anthropic(
                        &mut s.astate,
                        &chunk,
                        &echo,
                        &mut s.acc,
                    ));
                }
                (out, false)
            };
            let on_finish = |_s: &mut AnthropicFromGeminiRelay| Vec::new();
            let st = relay_sse(
                upstream,
                &tx,
                AnthropicFromGeminiRelay::default(),
                on_line,
                on_finish,
            )
            .await;
            StreamLog::new(
                &db_path,
                &appv,
                provider_id,
                key_id,
                latency_ms,
                status.as_u16(),
            )
            .write(&logm, &st.acc);
        });
        return sse_response(builder, rx);
    }
    if f.kind == TransKind::ResponsesToAnthropic {
        // Anthropic 上游 → Codex：Anthropic 事件 → Chat chunk → Responses 事件
        let appv = f.req.app.to_string();
        let echo = f.req.model_raw.to_string();
        let logm = log_fallback.clone();
        let customs = custom_tools.clone();
        tokio::spawn(async move {
            let on_line = |s: &mut ResponsesFromAnthropicRelay, t: &str| {
                anthropic_line_to_responses(t, s, &echo, &customs)
            };
            let on_finish = |s: &mut ResponsesFromAnthropicRelay| {
                crate::translate::responses_stream_finish(
                    &mut s.tstate,
                    &echo,
                    &s.acc,
                    &customs,
                )
            };
            let st = relay_sse(
                upstream,
                &tx,
                ResponsesFromAnthropicRelay::default(),
                on_line,
                on_finish,
            )
            .await;
            StreamLog::new(
                &db_path,
                &appv,
                provider_id,
                key_id,
                latency_ms,
                status.as_u16(),
            )
            .write(&logm, &st.acc);
        });
        return sse_response(builder, rx);
    }
    if f.kind == TransKind::ChatToAnthropic {
        // Anthropic 上游 → Chat 客戶端
        let appv = f.req.app.to_string();
        let logm = log_fallback.clone();
        tokio::spawn(async move {
            let on_line = |s: &mut ChatToAnthropicRelay, t: &str| {
                anthropic_line_to_chat(t, &mut s.astate, &mut s.acc)
            };
            let on_finish = |_s: &mut ChatToAnthropicRelay| Vec::new();
            let st = relay_sse(
                upstream,
                &tx,
                ChatToAnthropicRelay::default(),
                on_line,
                on_finish,
            )
            .await;
            StreamLog::new(
                &db_path,
                &appv,
                provider_id,
                key_id,
                latency_ms,
                status.as_u16(),
            )
            .write(&logm, &st.acc);
        });
        return sse_response(builder, rx);
    }
    if f.kind == TransKind::ResponsesToChat {
        // Responses 流式：chat chunk → Responses 事件；流結束補發 completed/failed
        let appv = f.req.app.to_string();
        let echo = f.req.model_raw.to_string();
        let logm = log_fallback.clone();
        let customs = custom_tools.clone();
        tokio::spawn(async move {
            let on_line = |s: &mut ResponsesRelay, t: &str| {
                responses_line_events(t, &mut s.tstate, &echo, &mut s.acc, &customs)
            };
            let on_finish = |s: &mut ResponsesRelay| {
                crate::translate::responses_stream_finish(
                    &mut s.tstate,
                    &echo,
                    &s.acc,
                    &customs,
                )
            };
            let st =
                relay_sse(upstream, &tx, ResponsesRelay::default(), on_line, on_finish)
                    .await;
            StreamLog::new(
                &db_path,
                &appv,
                provider_id,
                key_id,
                latency_ms,
                status.as_u16(),
            )
            .write(&logm, &st.acc);
        });
        return sse_response(builder, rx);
    }
    if f.translated {
        let appv = f.req.app.to_string();
        let echo = f.req.model_raw.to_string();
        let logm = log_fallback.clone();
        tokio::spawn(async move {
            let on_line = |s: &mut AnthropicRelay, t: &str| {
                let Some(d) = t.strip_prefix("data:") else {
                    return (vec![], false);
                };
                let d = d.trim_start();
                if d.is_empty() || d == "[DONE]" {
                    return (vec![], false);
                }
                let Ok(v) = serde_json::from_str::<serde_json::Value>(d) else {
                    return (vec![], false);
                };
                // 上游錯誤 → Anthropic error 事件並收流
                if let Some(err) = v.get("error") {
                    let err = if err.is_null() { v.clone() } else { err.clone() };
                    return (vec![crate::translate::anthropic_error_line(&err)], true);
                }
                (
                    crate::translate::openai_chunk_to_anthropic(
                        &mut s.tstate,
                        &v,
                        &echo,
                        &mut s.acc,
                    ),
                    false,
                )
            };
            let on_finish = |_s: &mut AnthropicRelay| Vec::new();
            let st =
                relay_sse(upstream, &tx, AnthropicRelay::default(), on_line, on_finish)
                    .await;
            StreamLog::new(
                &db_path,
                &appv,
                provider_id,
                key_id,
                latency_ms,
                status.as_u16(),
            )
            .write(&logm, &st.acc);
        });
        return sse_response(builder, rx);
    }
    let appv = f.req.app.to_string();
    let mr = f.req.model_raw.to_string();
    tokio::spawn(async move {
        let mut acc = SseAcc::new();
        let mut stream = upstream.bytes_stream();
        use tokio_stream::StreamExt;
        while let Some(item) = stream.next().await {
            match item {
                Ok(chunk) => {
                    acc.feed(&chunk);
                    if tx.send(Ok(chunk)).await.is_err() {
                        break;
                    }
                }
                Err(e) => {
                    let _ = tx.send(Err(axum::Error::new(e))).await;
                    break;
                }
            }
        }
        acc.finish();
        let u = acc.usage;
        let model = u.model.clone().unwrap_or(mr);
        StreamLog::new(
            &db_path,
            &appv,
            provider_id,
            key_id,
            latency_ms,
            status.as_u16(),
        )
        .write(&model, &u);
    });
    sse_response(builder, rx)
}
