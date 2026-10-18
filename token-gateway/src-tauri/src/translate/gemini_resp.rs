//! Gemini `generateContent` 回包 → OpenAI Chat Completions（非流式與流式）。
//!
//! 對應 `gemini_req.rs` 的另一半。產出的是**合成的 Chat chunk／回包**，
//! 所以它同時服務兩種客戶端：
//!
//! - Chat 客戶端：直接序列化成 SSE
//! - Codex（Responses）：丟進既有的 `chat_chunk_to_responses` / `openai_to_responses`
//!
//! ## 串流的累積問題
//!
//! Gemini 的 `streamGenerateContent` 每個 chunk 是**完整的
//! `GenerateContentResponse`**，不是 Chat 那種 delta。實測不同版本對
//! `parts[].text` 的語意不一致（有的送增量、有的送累積）。
//! [`GeminiToChatState`] 因此**兩種都容忍**：新文字若以已累積的開頭，就只送後綴。

use super::*;
use serde_json::json;

/// Gemini `finishReason` → Chat `finish_reason`。
fn finish_reason_to_chat(r: &str) -> &'static str {
    match r {
        "MAX_TOKENS" => "length",
        "SAFETY" | "RECITATION" | "BLOCKLIST" | "PROHIBITED_CONTENT" => "content_filter",
        // Gemini 用 STOP 表示正常結束；有 functionCall 時我們在下面自己改成 tool_calls
        _ => "stop",
    }
}

/// 從一個 `GenerateContentResponse` 取出 `(文字, 工具呼叫, finishReason, usage)`。
#[allow(clippy::type_complexity)]
fn read_candidate(resp: &Value) -> (String, Vec<Value>, Option<String>, (i64, i64)) {
    let mut text = String::new();
    let mut calls: Vec<Value> = vec![];
    let mut finish: Option<String> = None;
    if let Some(c) = resp
        .get("candidates")
        .and_then(|c| c.as_array())
        .and_then(|a| a.first())
    {
        if let Some(parts) = c
            .get("content")
            .and_then(|x| x.get("parts"))
            .and_then(|p| p.as_array())
        {
            for p in parts {
                if let Some(t) = p.get("text").and_then(|t| t.as_str()) {
                    text.push_str(t);
                }
                if let Some(fc) = p.get("functionCall") {
                    // `thoughtSignature` 必須在下一輪原樣送回（見
                    // `translate/mod.rs` 的 `SIGNATURE_SEP` 說明），
                    // 這裡把它夾進 id 一起帶出去。
                    let base = {
                        let gid = s(&fc["id"]);
                        if gid.is_empty() {
                            format!("call_{}", calls.len())
                        } else {
                            gid
                        }
                    };
                    let id = match p.get("thoughtSignature").and_then(|t| t.as_str()) {
                        Some(sig) if !sig.is_empty() => super::id_with_signature(&base, sig),
                        _ => base,
                    };
                    calls.push(json!({
                        "id": id,
                        "type": "function",
                        "function": {
                            "name": s(&fc["name"]),
                            "arguments": serde_json::to_string(&fc["args"])
                                .unwrap_or_else(|_| "{}".to_string()),
                        }
                    }));
                }
            }
        }
        if let Some(r) = c.get("finishReason").and_then(|r| r.as_str()) {
            finish = Some(r.to_string());
        }
    }
    let u = resp.get("usageMetadata").unwrap_or(&Value::Null);
    let in_tok = u
        .get("promptTokenCount")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let out_tok = u
        .get("candidatesTokenCount")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    (text, calls, finish, (in_tok, out_tok))
}

/// Gemini 非流式回包 → Chat 回包。`echo_model` 原樣回顯請求模型。
pub(crate) fn gemini_to_chat_resp(resp: &Value, echo_model: &str) -> Value {
    let (text, calls, finish, (in_tok, out_tok)) = read_candidate(resp);
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
    let fr = if msg.contains_key("tool_calls") {
        "tool_calls"
    } else {
        finish_reason_to_chat(finish.as_deref().unwrap_or("STOP"))
    };
    json!({
        "id": format!("chatcmpl-{}", crate::fsutil::now_ms()),
        "object": "chat.completion",
        "created": crate::fsutil::now_ms() / 1000,
        "model": echo_model,
        "choices": [{
            "index": 0,
            "message": Value::Object(msg),
            "finish_reason": fr,
        }],
        "usage": {
            "prompt_tokens": in_tok,
            "completion_tokens": out_tok,
            "total_tokens": in_tok + out_tok,
        }
    })
}

/// 流式狀態：把 Gemini 的 chunk 序列合成 Chat chunk。
#[derive(Debug, Default)]
pub(crate) struct GeminiToChatState {
    id: String,
    model: String,
    /// 已送出的文字（用來容忍「累積式」的 chunk）
    sent: String,
    role_sent: bool,
    tool_index: i64,
    finish: Option<String>,
    in_tok: i64,
    out_tok: i64,
    /// 已收到收尾（呼叫端據此收流並補 `[DONE]`）
    pub(crate) done: bool,
}

impl GeminiToChatState {
    fn base_chunk(&self, delta: Value, finish: Value) -> Value {
        json!({
            "id": self.id,
            "object": "chat.completion.chunk",
            "created": 0,
            "model": self.model,
            "choices": [{"index": 0, "delta": delta, "finish_reason": finish}],
        })
    }
}

/// 一個 Gemini 串流 chunk → 零到多個 Chat chunk。
///
/// Gemini 的 SSE `data:` 就是一個完整的 `GenerateContentResponse`，
/// 沒有 `type` 欄位可以分派 —— 全部靠「有沒有 candidates／usageMetadata」判斷。
pub(crate) fn gemini_chunk_to_chat_chunks(
    st: &mut GeminiToChatState,
    chunk: &Value,
) -> Vec<Value> {
    let mut out: Vec<Value> = vec![];
    let (text, calls, finish, (in_tok, out_tok)) = read_candidate(chunk);
    if st.id.is_empty() {
        st.id = format!("chatcmpl-{}", crate::fsutil::now_ms());
    }
    if st.model.is_empty() {
        st.model = s(&chunk["model"]);
    }
    if in_tok > 0 {
        st.in_tok = in_tok;
    }
    if out_tok > 0 {
        st.out_tok = out_tok;
    }
    if !st.role_sent {
        out.push(st.base_chunk(json!({"role": "assistant", "content": ""}), Value::Null));
        st.role_sent = true;
    }
    // 文字：容忍增量與累積兩種語意
    if !text.is_empty() {
        let delta = if text.starts_with(&st.sent) {
            text[st.sent.len()..].to_string()
        } else {
            text.clone()
        };
        if !delta.is_empty() {
            st.sent.push_str(&delta);
            out.push(st.base_chunk(json!({"content": delta}), Value::Null));
        }
    }
    // 工具呼叫：Gemini 一次送完整的 functionCall（不是片段）
    for c in calls {
        out.push(st.base_chunk(
            json!({"tool_calls": [{
                "index": st.tool_index,
                "id": c["id"],
                "type": "function",
                "function": {
                    "name": c["function"]["name"],
                    "arguments": c["function"]["arguments"],
                }
            }]}),
            Value::Null,
        ));
        st.tool_index += 1;
    }
    if let Some(r) = finish {
        st.finish = Some(if st.tool_index > 0 {
            "tool_calls".to_string()
        } else {
            finish_reason_to_chat(&r).to_string()
        });
        // Gemini 在最後一個 chunk 帶 finishReason 與 usageMetadata → 收尾
        out.push(st.base_chunk(
            json!({}),
            Value::String(st.finish.clone().unwrap_or_else(|| "stop".to_string())),
        ));
        let mut last = st.base_chunk(json!({}), Value::Null);
        last["usage"] = json!({
            "prompt_tokens": st.in_tok,
            "completion_tokens": st.out_tok,
            "total_tokens": st.in_tok + st.out_tok,
        });
        out.push(last);
        st.done = true;
    }
    out
}

/// Gemini 的錯誤物件 → Chat 的 error 物件。
pub(crate) fn gemini_error_to_chat(err: &Value) -> Value {
    let e = err.get("error").unwrap_or(err);
    json!({
        "error": {
            "message": s(&e["message"]),
            "type": s(&e["status"]),
            "code": e.get("code").cloned().unwrap_or(Value::Null),
        }
    })
}
