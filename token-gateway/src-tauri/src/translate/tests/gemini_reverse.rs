//! OpenAI → Gemini 反向翻譯（2026-10-11 補的那一半）。

use super::*;
use serde_json::json;

/// Chat 請求 → Gemini：system 拉成 `systemInstruction`、角色換成 `model`、
/// 工具結果包成 `functionResponse`、模型名進路徑。
#[test]
fn chat_request_becomes_gemini_shape() {
    let req = json!({
        "model": "m",
        "messages": [
            {"role": "system", "content": "sys"},
            {"role": "user", "content": "hi"},
            {"role": "assistant", "content": null, "tool_calls": [
                {"id": "t1", "type": "function",
                 "function": {"name": "get", "arguments": "{\"a\":1}"}}]},
            {"role": "tool", "name": "get", "tool_call_id": "t1", "content": "r1"}
        ],
        "tools": [{"type": "function", "function": {
            "name": "get", "description": "d", "parameters": {"type": "object"}}}],
        "tool_choice": "required",
        "temperature": 0.5,
        "max_tokens": 128
    });
    let (out, path) = chat_to_gemini(&req, "gemini-2.5-flash", &[]);

    assert_eq!(path, "/models/gemini-2.5-flash:generateContent");
    assert_eq!(out["systemInstruction"]["parts"][0]["text"], "sys");
    let c = out["contents"].as_array().unwrap();
    assert_eq!(c.len(), 3, "{c:?}");
    assert_eq!(c[0]["role"], "user");
    assert_eq!(c[0]["parts"][0]["text"], "hi");
    // assistant → role "model"（Gemini 沒有 assistant）
    assert_eq!(c[1]["role"], "model");
    assert_eq!(c[1]["parts"][0]["functionCall"]["name"], "get");
    assert_eq!(c[1]["parts"][0]["functionCall"]["args"]["a"], 1);
    // 工具結果 → **user** 的一個 functionResponse part（Gemini 沒有 tool 角色）
    assert_eq!(c[2]["role"], "user");
    assert_eq!(c[2]["parts"][0]["functionResponse"]["name"], "get");
    assert_eq!(c[2]["parts"][0]["functionResponse"]["response"]["content"], "r1");
    // 工具宣告與 toolConfig
    assert_eq!(out["tools"][0]["functionDeclarations"][0]["name"], "get");
    assert_eq!(
        out["toolConfig"]["functionCallingConfig"]["mode"], "ANY",
        "required → ANY"
    );
    // generationConfig
    assert_eq!(out["generationConfig"]["temperature"], 0.5);
    assert_eq!(out["generationConfig"]["maxOutputTokens"], 128);
}

/// 串流是**另一個動作名**，而且要 `alt=sse` 才會回 SSE。
#[test]
fn streaming_uses_the_stream_action_and_alt_sse() {
    let (_, p1) = chat_to_gemini(&json!({"model": "m", "messages": []}), "g", &[]);
    assert_eq!(p1, "/models/g:generateContent");
    let (_, p2) = chat_to_gemini(
        &json!({"model": "m", "messages": [], "stream": true}),
        "g",
        &[],
    );
    assert_eq!(p2, "/models/g:streamGenerateContent?alt=sse");
    // 使用者填 models/ 前綴不該變成 /models/models/
    let (_, p3) = chat_to_gemini(&json!({"model": "m", "messages": []}), "models/g", &[]);
    assert_eq!(p3, "/models/g:generateContent");
}

/// 思考強度 → `thinkingConfig`，且與 Anthropic 那份映射**同構**。
#[test]
fn reasoning_effort_maps_to_thinking_config() {
    let allowed = vec!["high".to_string()];
    let (out, _) = chat_to_gemini(
        &json!({"model": "m", "messages": [], "reasoning_effort": "high"}),
        "g",
        &allowed,
    );
    assert_eq!(out["generationConfig"]["thinkingConfig"]["thinkingBudget"], 16384);
    // 與 Anthropic 共用同一份映射 → 同一個檔位得到同一個預算
    assert_eq!(
        out["generationConfig"]["thinkingConfig"]["thinkingBudget"],
        thinking_budget("high").unwrap()
    );
    // 不在允許清單裡就不啟用
    let (out2, _) = chat_to_gemini(
        &json!({"model": "m", "messages": [], "reasoning_effort": "max"}),
        "g",
        &allowed,
    );
    assert!(out2["generationConfig"].get("thinkingConfig").is_none());
}

/// Gemini 回包 → Chat：parts 文字、functionCall → tool_calls、finishReason 映射。
#[test]
fn gemini_response_becomes_chat_shape() {
    let resp = json!({
        "candidates": [{
            "content": {"role": "model", "parts": [
                {"text": "答案"},
                {"functionCall": {"name": "get", "args": {"a": 1}}}
            ]},
            "finishReason": "STOP"
        }],
        "usageMetadata": {"promptTokenCount": 7, "candidatesTokenCount": 3}
    });
    let out = gemini_to_chat_resp(&resp, "echo");
    assert_eq!(out["object"], "chat.completion");
    assert_eq!(out["model"], "echo");
    assert_eq!(out["choices"][0]["message"]["content"], "答案");
    assert_eq!(
        out["choices"][0]["message"]["tool_calls"][0]["function"]["name"],
        "get"
    );
    assert_eq!(
        out["choices"][0]["message"]["tool_calls"][0]["function"]["arguments"],
        "{\"a\":1}"
    );
    assert_eq!(
        out["choices"][0]["finish_reason"], "tool_calls",
        "有工具呼叫時就該是 tool_calls"
    );
    assert_eq!(out["usage"]["prompt_tokens"], 7);
    assert_eq!(out["usage"]["completion_tokens"], 3);
    assert_eq!(out["usage"]["total_tokens"], 10);

    // MAX_TOKENS → length
    let r2 = json!({"candidates": [{"content": {"parts": [{"text": "x"}]},
                                    "finishReason": "MAX_TOKENS"}]});
    assert_eq!(gemini_to_chat_resp(&r2, "e")["choices"][0]["finish_reason"], "length");
    // SAFETY → content_filter
    let r3 = json!({"candidates": [{"content": {"parts": []}, "finishReason": "SAFETY"}]});
    assert_eq!(
        gemini_to_chat_resp(&r3, "e")["choices"][0]["finish_reason"],
        "content_filter"
    );
}

/// 串流：Gemini chunk → Chat chunk，**增量與累積兩種語意都要容忍**。
#[test]
fn gemini_chunks_become_chat_chunks_tolerating_both_text_semantics() {
    // 增量式
    let mut st = GeminiToChatState::default();
    let mut all: Vec<serde_json::Value> = vec![];
    for c in [
        json!({"candidates": [{"content": {"parts": [{"text": "hello "}]}}]}),
        json!({"candidates": [{"content": {"parts": [{"text": "world"}]}}]}),
        json!({"candidates": [{"content": {"parts": []}, "finishReason": "STOP"}],
               "usageMetadata": {"promptTokenCount": 4, "candidatesTokenCount": 9}}),
    ] {
        all.extend(gemini_chunk_to_chat_chunks(&mut st, &c));
    }
    assert!(st.done);
    let text: String = all
        .iter()
        .filter_map(|c| c["choices"][0]["delta"]["content"].as_str())
        .collect();
    assert_eq!(text, "hello world");
    assert!(all.iter().any(|c| c["choices"][0]["finish_reason"] == "stop"));
    assert!(all.iter().any(|c| c["usage"]["completion_tokens"] == 9));

    // 累積式：同一段文字重送兩次，第二次只該送出後綴（空）
    let mut st2 = GeminiToChatState::default();
    let mut out2: Vec<serde_json::Value> = vec![];
    for c in [
        json!({"candidates": [{"content": {"parts": [{"text": "ab"}]}}]}),
        json!({"candidates": [{"content": {"parts": [{"text": "abc"}]}}]}),
    ] {
        out2.extend(gemini_chunk_to_chat_chunks(&mut st2, &c));
    }
    let t2: String = out2
        .iter()
        .filter_map(|c| c["choices"][0]["delta"]["content"].as_str())
        .collect();
    assert_eq!(t2, "abc", "累積式不該變成 ababc");
}

/// 串流裡的功能呼叫：Gemini 一次送完整呼叫（不是片段）。
#[test]
fn gemini_streaming_function_call_becomes_tool_call() {
    let mut st = GeminiToChatState::default();
    let mut all: Vec<serde_json::Value> = vec![];
    for c in [
        json!({"candidates": [{"content": {"parts": [
            {"functionCall": {"name": "get", "args": {"a": 1}}}]}}]}),
        json!({"candidates": [{"content": {"parts": []}, "finishReason": "STOP"}]}),
    ] {
        all.extend(gemini_chunk_to_chat_chunks(&mut st, &c));
    }
    let tc: Vec<&serde_json::Value> = all
        .iter()
        .filter_map(|c| c["choices"][0]["delta"]["tool_calls"].as_array())
        .flatten()
        .collect();
    assert_eq!(tc.len(), 1, "{tc:?}");
    assert_eq!(tc[0]["function"]["name"], "get");
    assert_eq!(tc[0]["function"]["arguments"], "{\"a\":1}");
    // 有工具呼叫 → finish_reason 要是 tool_calls
    assert!(all
        .iter()
        .any(|c| c["choices"][0]["finish_reason"] == "tool_calls"));
}

/// Gemini 的錯誤物件 → Chat 的 error 物件。
#[test]
fn gemini_error_becomes_chat_error() {
    let e = json!({"error": {"code": 429, "message": "quota", "status": "RESOURCE_EXHAUSTED"}});
    let out = gemini_error_to_chat(&e);
    assert_eq!(out["error"]["message"], "quota");
    assert_eq!(out["error"]["type"], "RESOURCE_EXHAUSTED");
    assert_eq!(out["error"]["code"], 429);
}
