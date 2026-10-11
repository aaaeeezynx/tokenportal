//! Anthropic Messages 回包 → OpenAI Chat Completions（非流式與流式）。
//!
//! 對應 `anthropic_req.rs` 的另一半。產出的是**合成的 Chat chunk／回包**，
//! 所以它同時服務兩種客戶端：
//!
//! - Chat 客戶端：直接序列化成 SSE
//! - Codex（Responses）：丟進既有的 `chat_chunk_to_responses` / `openai_to_responses`
//!
//! ## 思考區塊刻意丟棄
//!
//! Anthropic 的 `thinking` / `redacted_thinking` 區塊**不轉成** `reasoning_content`
//! 或 Responses 的 reasoning item。理由：Codex 會把 reasoning item 回送，而上游
//! 認不得自己沒發過的 id —— 那正是 2026-10-10 `muse-spark` 那個
//! `Referenced reasoning item … was not found` 的形狀。既有的 Claude Code 路徑
//! （`anthropic_to_openai`）也是丟棄，這裡保持一致。

use super::*;
use serde_json::json;

/// Anthropic `stop_reason` → Chat `finish_reason`。
fn stop_reason_to_finish(r: &str) -> &'static str {
    match r {
        "max_tokens" => "length",
        "tool_use" => "tool_calls",
        "refusal" => "content_filter",
        _ => "stop",
    }
}

/// Anthropic 非流式回包 → Chat 回包。`echo_model` 原樣回顯請求模型。
pub(crate) fn anthropic_to_chat_resp(resp: &Value, echo_model: &str) -> Value {
    let mut text = String::new();
    let mut calls: Vec<Value> = vec![];
    if let Some(blocks) = resp.get("content").and_then(|c| c.as_array()) {
        for b in blocks {
            match s(&b["type"]).as_str() {
                "text" => text.push_str(&s(&b["text"])),
                "tool_use" => calls.push(json!({
                    "id": s(&b["id"]),
                    "type": "function",
                    "function": {
                        "name": s(&b["name"]),
                        "arguments": serde_json::to_string(&b["input"])
                            .unwrap_or_else(|_| "{}".to_string()),
                    }
                })),
                // thinking / redacted_thinking：丟棄（見檔頭）
                _ => {}
            }
        }
    }
    let mut msg = serde_json::Map::new();
    msg.insert("role".to_string(), Value::String("assistant".to_string()));
    msg.insert(
        "content".to_string(),
        if text.is_empty() {
            Value::Null
        } else {
            Value::String(text)
        },
    );
    if !calls.is_empty() {
        msg.insert("tool_calls".to_string(), Value::Array(calls));
    }
    let null = Value::Null;
    let u = resp.get("usage").unwrap_or(&null);
    let in_tok = u.get("input_tokens").and_then(|v| v.as_i64()).unwrap_or(0);
    let out_tok = u.get("output_tokens").and_then(|v| v.as_i64()).unwrap_or(0);
    json!({
        "id": format!("chatcmpl-{}", s(&resp["id"]).trim_start_matches("msg_")),
        "object": "chat.completion",
        // Anthropic 的回包沒有 created；用現在時間，客戶端多半不看這個欄位
        "created": crate::fsutil::now_ms() / 1000,
        "model": echo_model,
        "choices": [{
            "index": 0,
            "message": Value::Object(msg),
            "finish_reason": stop_reason_to_finish(&s(&resp["stop_reason"])),
        }],
        "usage": {
            "prompt_tokens": in_tok,
            "completion_tokens": out_tok,
            "total_tokens": in_tok + out_tok,
        }
    })
}

/// 流式狀態：把 Anthropic 的事件序列合成 Chat chunk。
#[derive(Debug, Default)]
pub(crate) struct AnthropicToChatState {
    id: String,
    model: String,
    created: i64,
    role_sent: bool,
    /// 目前開啟中的 content block（`text` 或 `tool_use`）
    block: Option<String>,
    /// 這個 block 在 Chat 的 `tool_calls` 陣列裡的序號
    tool_index: i64,
    /// 已經送過 tool_calls 的第一個 chunk（要帶 id/name，後續只帶 arguments）
    tool_opened: bool,
    finish: Option<String>,
    in_tok: i64,
    out_tok: i64,
    /// 已收到 `message_stop`（呼叫端據此收流並補 `[DONE]`）
    pub(crate) done: bool,
}

impl AnthropicToChatState {
    fn base_chunk(&self, delta: Value, finish: Value) -> Value {
        json!({
            "id": self.id,
            "object": "chat.completion.chunk",
            "created": self.created,
            "model": self.model,
            "choices": [{"index": 0, "delta": delta, "finish_reason": finish}],
        })
    }
}

/// 一個 Anthropic SSE 事件 → 零到多個 Chat chunk。
///
/// 事件種類（`data.type`）：`message_start` / `content_block_start` /
/// `content_block_delta` / `content_block_stop` / `message_delta` /
/// `message_stop` / `ping` / `error`。未知的一律忽略（不讓串流中斷）。
pub(crate) fn anthropic_event_to_chat_chunks(
    st: &mut AnthropicToChatState,
    ev: &Value,
) -> Vec<Value> {
    let mut out: Vec<Value> = vec![];
    match s(&ev["type"]).as_str() {
        "message_start" => {
            let m = &ev["message"];
            st.id = format!(
                "chatcmpl-{}",
                s(&m["id"]).trim_start_matches("msg_")
            );
            st.model = s(&m["model"]);
            // Anthropic 不送 created_at；用現在時間，客戶端多半不看
            st.created = crate::fsutil::now_ms() / 1000;
            st.in_tok = m["usage"]["input_tokens"].as_i64().unwrap_or(0);
            st.out_tok = m["usage"]["output_tokens"].as_i64().unwrap_or(0);
        }
        "content_block_start" => {
            let b = &ev["content_block"];
            st.tool_opened = false;
            match s(&b["type"]).as_str() {
                "tool_use" => {
                    st.block = Some("tool_use".to_string());
                    // 第一個 chunk 帶 id/name/空 arguments（OpenAI 的形狀）
                    out.push(st.base_chunk(
                        json!({"tool_calls": [{
                            "index": st.tool_index,
                            "id": s(&b["id"]),
                            "type": "function",
                            "function": {"name": s(&b["name"]), "arguments": ""}
                        }]}),
                        Value::Null,
                    ));
                    st.tool_opened = true;
                }
                "text" => {
                    st.block = Some("text".to_string());
                    // Anthropic 的 text block 起始就帶完整文字（多半是空的）
                    let t = s(&b["text"]);
                    if !t.is_empty() {
                        out.push(st.base_chunk(json!({"content": t}), Value::Null));
                    }
                }
                // thinking / redacted_thinking：丟棄（見檔頭）
                _ => st.block = None,
            }
        }
        "content_block_delta" => {
            let d = &ev["delta"];
            match s(&d["type"]).as_str() {
                "text_delta" => {
                    let t = s(&d["text"]);
                    if !t.is_empty() {
                        out.push(st.base_chunk(json!({"content": t}), Value::Null));
                    }
                }
                "input_json_delta" => {
                    // Anthropic 送的是**片段 JSON**；Chat 的 arguments 也是片段，
                    // 所以原樣搬過去即可（累積由客戶端做）。
                    let p = s(&d["partial_json"]);
                    if st.tool_opened {
                        out.push(st.base_chunk(
                            json!({"tool_calls": [{
                                "index": st.tool_index,
                                "function": {"arguments": p}
                            }]}),
                            Value::Null,
                        ));
                    }
                }
                // thinking_delta / signature_delta：丟棄
                _ => {}
            }
        }
        "content_block_stop" => {
            if st.block.as_deref() == Some("tool_use") {
                st.tool_index += 1;
            }
            st.block = None;
        }
        "message_delta" => {
            if let Some(r) = ev["delta"]["stop_reason"].as_str() {
                st.finish = Some(stop_reason_to_finish(r).to_string());
            }
            if let Some(n) = ev["usage"]["output_tokens"].as_i64() {
                st.out_tok = n;
            }
        }
        "message_stop" => {
            // 收尾：先發 role（若整段都沒有內容，客戶端仍需要一個合法的 chunk），
            // 再發 finish_reason，最後帶 usage 的那個 chunk。
            if !st.role_sent {
                out.push(st.base_chunk(json!({"role": "assistant", "content": ""}), Value::Null));
                st.role_sent = true;
            }
            let f = st.finish.clone().unwrap_or_else(|| "stop".to_string());
            out.push(st.base_chunk(json!({}), Value::String(f)));
            let mut last = st.base_chunk(json!({}), Value::Null);
            last["usage"] = json!({
                "prompt_tokens": st.in_tok,
                "completion_tokens": st.out_tok,
                "total_tokens": st.in_tok + st.out_tok,
            });
            out.push(last);
            st.done = true;
        }
        _ => {}
    }
    out
}

/// Anthropic 的 error 事件 → Chat 的 error 物件。
pub(crate) fn anthropic_error_to_chat(err: &Value) -> Value {
    json!({
        "error": {
            "message": s(&err["error"]["message"]),
            "type": s(&err["error"]["type"]),
            "code": Value::Null,
        }
    })
}
