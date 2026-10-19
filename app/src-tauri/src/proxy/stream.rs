//! SSE 轉送：逐行中繼、事件形狀轉換、串流結束記帳。
//!
//! `relay_sse` 是泛型骨架，把「逐行讀取」與「每行怎麼處理」分開；
//! `on_line` 決定要不要繼續、`on_finish` 決定流結束要補什麼。

use super::*;
use crate::translate::SseUsage;

/// 處理一行上游 SSE（Responses 翻譯分支）：回傳（要送出的事件, 是否停止讀流）。
/// - `[DONE]` → 標記正常收尾並停止；
/// - `{"error": …}` → 發 `response.failed` 並停止（不再靜默當 completed）；
/// - 其餘 → chat chunk → Responses 事件。
pub(super) fn responses_line_events(
    t: &str,
    tstate: &mut crate::translate::ResponsesStreamState,
    echo: &str,
    acc: &mut SseUsage,
    customs: &[String],
) -> (Vec<String>, bool) {
    let Some(d) = t.strip_prefix("data:") else {
        return (vec![], false);
    };
    let d = d.trim_start();
    if d.is_empty() {
        return (vec![], false);
    }
    if d == "[DONE]" {
        tstate.note_done();
        return (vec![], true);
    }
    let Ok(v) = serde_json::from_str::<serde_json::Value>(d) else {
        return (vec![], false);
    };
    if let Some(err) = v.get("error") {
        let err = if err.is_null() { v.clone() } else { err.clone() };
        return (crate::translate::responses_stream_fail(tstate, echo, &err), true);
    }
    (
        crate::translate::chat_chunk_to_responses(tstate, &v, echo, acc, customs),
        false,
    )
}
/// SSE 轉送的統一骨架：逐行讀上游、交給 `on_line` 轉譯、結束時由 `on_finish`
/// 補收尾事件，最後把狀態交還呼叫端（供寫用量日誌）。
///
/// 三個串流分支原本各自手寫這套 buffer/drain/收流邏輯（各約 60–70 行），
/// 且尾行處理與斷線處理的細節已經開始分歧。這裡統一：`on_line` 回傳
/// `(要送出的內容, 是否收流)`，非 `data:` 行或無法解析的行只要回空 vec 即可。
pub(super) async fn relay_sse<S, F, G>(
    upstream: reqwest::Response,
    tx: &tokio::sync::mpsc::Sender<Result<axum::body::Bytes, axum::Error>>,
    mut state: S,
    mut on_line: F,
    mut on_finish: G,
) -> S
where
    F: FnMut(&mut S, &str) -> (Vec<String>, bool),
    G: FnMut(&mut S) -> Vec<String>,
{
    use tokio_stream::StreamExt;
    let mut buf = String::new();
    let mut closed = false;
    let mut stream = upstream.bytes_stream();
    'outer: while let Some(item) = stream.next().await {
        match item {
            Ok(chunk) => {
                buf.push_str(&String::from_utf8_lossy(&chunk));
                while let Some(nl) = buf.find('\n') {
                    let line: String = buf.drain(..=nl).collect();
                    let t = line.trim_end_matches(['\r', '\n']);
                    let (outs, stop) = on_line(&mut state, t);
                    for o in outs {
                        if tx.send(Ok(axum::body::Bytes::from(o))).await.is_err() {
                            closed = true;
                            break 'outer;
                        }
                    }
                    if stop {
                        closed = true;
                        break 'outer;
                    }
                }
            }
            Err(e) => {
                let _ = tx.send(Err(axum::Error::new(e))).await;
                closed = true;
                break;
            }
        }
    }
    // 尾端沒有換行的殘留行（正常 SSE 以空行結尾，這裡通常空操作）
    if !closed {
        let rest = buf.trim_end_matches(['\r', '\n']).to_string();
        if !rest.is_empty() {
            let (outs, _) = on_line(&mut state, &rest);
            for o in outs {
                let _ = tx.send(Ok(axum::body::Bytes::from(o))).await;
            }
        }
    }
    for o in on_finish(&mut state) {
        let _ = tx.send(Ok(axum::body::Bytes::from(o))).await;
    }
    state
}
/// 串流結束後寫用量日誌所需的固定脈絡（三個串流分支共用）。
pub(super) struct StreamLog<'a> {
    pub(super) db_path: &'a PathBuf,
    pub(super) app: &'a str,
    pub(super) provider_id: i64,
    pub(super) key_id: Option<i64>,
    pub(super) latency_ms: i64,
    pub(super) status: u16,
}

impl<'a> StreamLog<'a> {
    pub(super) fn new(
        db_path: &'a PathBuf,
        app: &'a str,
        provider_id: i64,
        key_id: Option<i64>,
        latency_ms: i64,
        status: u16,
    ) -> Self {
        Self {
            db_path,
            app,
            provider_id,
            key_id,
            latency_ms,
            status,
        }
    }

    /// 寫一筆串流請求的用量日誌並累加 Key 額度。
    /// （原本三個分支各手寫約 20 行的同款程式碼，只有 model/usage 來源不同。）
    pub(super) fn write(&self, model: &str, acc: &SseUsage) {
        let norm = normalize_model(model);
        let Ok(conn) = open_conn(self.db_path) else {
            return;
        };
        let ts = now_ms();
        let rates = crate::usage::resolve_rates(&conn, Some(self.provider_id), &norm, ts);
        let cost = crate::usage::rate_cost(
            &rates,
            acc.in_tok,
            acc.out_tok,
            acc.cache_read,
            acc.cache_write,
        );
        let _ = insert_log(
            &conn,
            ts,
            self.key_id,
            self.app,
            Some(self.provider_id),
            model,
            &norm,
            acc,
            cost,
            self.latency_ms,
            self.status as i64,
            true,
        );
        if let Some(kid) = self.key_id {
            let _ = keys::add_used(&conn, kid, acc.total());
        }
    }
}
/// 把 mpsc 接收端包成 SSE 響應；建 body 失敗時回 500。
pub(super) fn sse_response(
    builder: axum::http::response::Builder,
    rx: tokio::sync::mpsc::Receiver<Result<axum::body::Bytes, axum::Error>>,
) -> Response {
    let stream_body = Body::from_stream(tokio_stream::wrappers::ReceiverStream::new(rx));
    builder
        .body(stream_body)
        .unwrap_or_else(|_| err_json(StatusCode::INTERNAL_SERVER_ERROR, "構造響應失敗".into()))
}
/// Anthropic 上游 → **Chat 客戶端**：Anthropic 事件 → 合成 Chat chunk → SSE。
///
/// Anthropic 的 SSE `data:` 內容自帶 `"type"`，所以不必看 `event:` 行。
pub(super) fn anthropic_line_to_chat(
    t: &str,
    state: &mut crate::translate::AnthropicToChatState,
    acc: &mut SseUsage,
) -> (Vec<String>, bool) {
    let Some(d) = t.strip_prefix("data:") else {
        return (vec![], false);
    };
    let d = d.trim_start();
    if d.is_empty() {
        return (vec![], false);
    }
    let Ok(v) = serde_json::from_str::<serde_json::Value>(d) else {
        return (vec![], false);
    };
    // Anthropic 的 error 事件 → Chat 的 error 物件，收流
    if v.get("error").is_some() {
        let e = crate::translate::anthropic_error_to_chat(&v);
        return (vec![format!("data: {e}\n\n")], true);
    }
    let chunks = crate::translate::anthropic_event_to_chat_chunks(state, &v);
    let mut out = vec![];
    for c in &chunks {
        if c.get("usage").is_some() {
            crate::translate::read_openai_usage(&c["usage"], acc);
        }
        out.push(format!("data: {c}\n\n"));
    }
    if state.done {
        out.push("data: [DONE]\n\n".to_string());
        return (out, true);
    }
    (out, false)
}

/// Anthropic 上游 → **Responses 客戶端（Codex）**：Anthropic 事件 → 合成 Chat
/// chunk → 餵給既有的 `chat_chunk_to_responses`。
///
/// 這正是「兩跳」的第二段 —— 重用既有、已被測過的 Responses 串流建構器，
/// 而不是再寫一份 Anthropic → Responses 的直接轉換。
pub(super) fn anthropic_line_to_responses(
    t: &str,
    state: &mut ResponsesFromAnthropicRelay,
    echo: &str,
    customs: &[String],
) -> (Vec<String>, bool) {
    let Some(d) = t.strip_prefix("data:") else {
        return (vec![], false);
    };
    let d = d.trim_start();
    if d.is_empty() {
        return (vec![], false);
    }
    let Ok(v) = serde_json::from_str::<serde_json::Value>(d) else {
        return (vec![], false);
    };
    if v.get("error").is_some() {
        let e = crate::translate::anthropic_error_to_chat(&v);
        return (
            crate::translate::responses_stream_fail(&mut state.tstate, echo, &e),
            true,
        );
    }
    let chunks = crate::translate::anthropic_event_to_chat_chunks(&mut state.astate, &v);
    let mut out = vec![];
    for c in &chunks {
        out.extend(crate::translate::chat_chunk_to_responses(
            &mut state.tstate,
            c,
            echo,
            &mut state.acc,
            customs,
        ));
    }
    // 收流後由 `on_finish` 補 completed/failed（Responses 分支的既有作法）
    (out, state.astate.done)
}

/// Gemini 上游 → **Chat 客戶端**：Gemini chunk → 合成 Chat chunk → SSE。
///
/// Gemini 的 SSE `data:` 就是一個完整的 `GenerateContentResponse`（沒有 `type`
/// 可分派），而且**沒有 `[DONE]` 結尾** —— 收尾靠 `on_finish` 補。
pub(super) fn gemini_line_to_chat(
    t: &str,
    state: &mut ChatFromGeminiRelay,
) -> (Vec<String>, bool) {
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
    if v.get("error").is_some() {
        let e = crate::translate::gemini_error_to_chat(&v);
        return (vec![format!("data: {e}\n\n")], true);
    }
    let chunks = crate::translate::gemini_chunk_to_chat_chunks(&mut state.astate, &v);
    let mut out = vec![];
    for c in &chunks {
        if c.get("usage").is_some() {
            crate::translate::read_openai_usage(&c["usage"], &mut state.acc);
        }
        out.push(format!("data: {c}\n\n"));
    }
    (out, state.astate.done)
}

/// Gemini 上游 → **Responses 客戶端（Codex）**：Gemini chunk → 合成 Chat chunk
/// → 餵給既有的 `chat_chunk_to_responses`。兩跳的第二段。
pub(super) fn gemini_line_to_responses(
    t: &str,
    state: &mut ResponsesFromGeminiRelay,
    echo: &str,
    customs: &[String],
) -> (Vec<String>, bool) {
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
    if v.get("error").is_some() {
        let e = crate::translate::gemini_error_to_chat(&v);
        return (
            crate::translate::responses_stream_fail(&mut state.tstate, echo, &e),
            true,
        );
    }
    let chunks = crate::translate::gemini_chunk_to_chat_chunks(&mut state.astate, &v);
    let mut out = vec![];
    for c in &chunks {
        out.extend(crate::translate::chat_chunk_to_responses(
            &mut state.tstate,
            c,
            echo,
            &mut state.acc,
            customs,
        ));
    }
    // 收流後由 `on_finish` 補 completed/failed
    (out, state.astate.done)
}

/// Responses 翻譯分支的逐行轉譯狀態。
#[derive(Default)]
pub(super) struct ResponsesRelay {
    pub(super) tstate: crate::translate::ResponsesStreamState,
    pub(super) acc: SseUsage,
}
/// Anthropic 翻譯分支的逐行轉譯狀態（**Anthropic 入站** → Chat 上游）。
#[derive(Default)]
pub(super) struct AnthropicRelay {
    pub(super) tstate: crate::translate::AnthropicStreamState,
    pub(super) acc: SseUsage,
}
/// Anthropic **上游** → Chat 客戶端（2026-10-10 補的反向）。
#[derive(Default)]
pub(super) struct ChatToAnthropicRelay {
    pub(super) astate: crate::translate::AnthropicToChatState,
    pub(super) acc: SseUsage,
}
/// Anthropic **上游** → Responses 客戶端（Codex）。兩跳的第二段。
#[derive(Default)]
pub(super) struct ResponsesFromAnthropicRelay {
    pub(super) astate: crate::translate::AnthropicToChatState,
    pub(super) tstate: crate::translate::ResponsesStreamState,
    pub(super) acc: SseUsage,
}
/// Gemini **上游** → Chat 客戶端（2026-10-11）。
#[derive(Default)]
pub(super) struct ChatFromGeminiRelay {
    pub(super) astate: crate::translate::GeminiToChatState,
    pub(super) acc: SseUsage,
}
/// Gemini **上游** → Responses 客戶端（Codex）。兩跳的第二段。
#[derive(Default)]
pub(super) struct ResponsesFromGeminiRelay {
    pub(super) astate: crate::translate::GeminiToChatState,
    pub(super) tstate: crate::translate::ResponsesStreamState,
    pub(super) acc: SseUsage,
}
