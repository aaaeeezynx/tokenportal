//! OpenAI → Anthropic 反向翻譯（2026-10-10 補的那一半）。

use super::*;
use serde_json::json;

/// 思考強度 → 延伸思考預算：**必須單調**，否則使用者選高檔位反而更少思考。
#[test]
fn thinking_budget_is_monotonic() {
    let order = ["minimal", "low", "medium", "high", "xhigh", "max"];
    let vals: Vec<i64> = order
        .iter()
        .map(|e| thinking_budget(e).unwrap_or_else(|| panic!("{e} 應有預算")))
        .collect();
    for w in vals.windows(2) {
        assert!(w[0] < w[1], "預算必須遞增：{vals:?}");
    }
    assert_eq!(vals[0], 1024, "Anthropic 的下限");
    // ultra / persistent 與 max 同級
    assert_eq!(thinking_budget("ultra"), thinking_budget("max"));
    assert_eq!(thinking_budget("persistent"), thinking_budget("max"));
    // 不認的值不啟用
    assert_eq!(thinking_budget("unset"), None);
    assert_eq!(thinking_budget(""), None);
    assert_eq!(thinking_budget("bogus"), None);
}

/// Chat 請求 → Anthropic：system 拉頂層、tool 結果併成一個 user 訊息、工具換
/// `input_schema`、`max_tokens` 補預設。
#[test]
fn chat_request_becomes_anthropic_shape() {
    let req = json!({
        "model": "m",
        "messages": [
            {"role": "system", "content": "sys"},
            {"role": "user", "content": "hi"},
            {"role": "assistant", "content": null, "tool_calls": [
                {"id": "t1", "type": "function",
                 "function": {"name": "get", "arguments": "{\"a\":1}"}}]},
            {"role": "tool", "tool_call_id": "t1", "content": "r1"},
            {"role": "tool", "tool_call_id": "t2", "content": "r2"}
        ],
        "tools": [{"type": "function", "function": {
            "name": "get", "description": "d", "parameters": {"type": "object"}}}],
        "tool_choice": "required",
        "temperature": 0.5
    });
    let out = chat_to_anthropic(&req, "actual", &[]);

    assert_eq!(out["model"], "actual");
    assert_eq!(out["system"], "sys", "system 要拉到頂層");
    // user → assistant(tool_use) → user(兩個 tool_result 併在一起)
    let msgs = out["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 3, "{msgs:?}");
    assert_eq!(msgs[0]["role"], "user");
    assert_eq!(msgs[0]["content"][0]["type"], "text");
    assert_eq!(msgs[1]["role"], "assistant");
    assert_eq!(msgs[1]["content"][0]["type"], "tool_use");
    assert_eq!(msgs[1]["content"][0]["input"]["a"], 1, "arguments 要還原成物件");
    assert_eq!(msgs[2]["role"], "user");
    let blocks = msgs[2]["content"].as_array().unwrap();
    assert_eq!(blocks.len(), 2, "連續的 tool 結果要併成同一個 user 訊息");
    assert_eq!(blocks[0]["type"], "tool_result");
    assert_eq!(blocks[0]["tool_use_id"], "t1");
    // 工具與 tool_choice
    assert_eq!(out["tools"][0]["name"], "get");
    assert_eq!(out["tools"][0]["input_schema"]["type"], "object");
    assert_eq!(out["tool_choice"]["type"], "any", "required → any");
    // max_tokens 必填：沒帶就用預設
    assert_eq!(out["max_tokens"], 8192);
    // 沒開思考時 temperature 照傳
    assert_eq!(out["temperature"], 0.5);
}

/// 開了延伸思考時：`budget_tokens < max_tokens`，且**丟掉** temperature／top_p
///（Anthropic 兩者互斥，留著會 400）。
#[test]
fn thinking_forces_budget_under_max_tokens_and_drops_sampling() {
    let allowed = vec!["high".to_string()];
    let req = json!({
        "model": "m",
        "messages": [{"role": "user", "content": "hi"}],
        "max_tokens": 1000,
        "temperature": 0.7,
        "top_p": 0.9,
        "reasoning_effort": "high"
    });
    let out = chat_to_anthropic(&req, "m", &allowed);
    let budget = out["thinking"]["budget_tokens"].as_i64().unwrap();
    let max = out["max_tokens"].as_i64().unwrap();
    assert_eq!(budget, 16384);
    assert!(budget < max, "budget {budget} 必須 < max_tokens {max}");
    assert_eq!(max, 16384 + 1024, "為了裝下預算而把上限往上調");
    assert!(out.get("temperature").is_none(), "思考與 temperature 互斥");
    assert!(out.get("top_p").is_none(), "思考與 top_p 互斥");

    // 沒被允許的檔位不開思考（與 Chat 路徑同一道閘門）
    let req2 = json!({"model": "m", "messages": [], "reasoning_effort": "max"});
    let out2 = chat_to_anthropic(&req2, "m", &allowed);
    assert!(out2.get("thinking").is_none(), "max 不在允許清單裡");
}

/// Anthropic 回包 → Chat：tool_use → tool_calls、stop_reason 映射、usage 換算。
#[test]
fn anthropic_response_becomes_chat_shape() {
    let resp = json!({
        "id": "msg_abc", "type": "message", "role": "assistant", "model": "claude",
        "content": [
            {"type": "thinking", "thinking": "祕密"},
            {"type": "text", "text": "答案"},
            {"type": "tool_use", "id": "t1", "name": "get", "input": {"a": 1}}
        ],
        "stop_reason": "tool_use",
        "usage": {"input_tokens": 7, "output_tokens": 3}
    });
    let out = anthropic_to_chat_resp(&resp, "echo-model");
    assert_eq!(out["object"], "chat.completion");
    assert_eq!(out["model"], "echo-model", "回顯請求模型");
    assert_eq!(out["choices"][0]["message"]["content"], "答案");
    assert_eq!(
        out["choices"][0]["message"]["tool_calls"][0]["function"]["name"],
        "get"
    );
    assert_eq!(
        out["choices"][0]["message"]["tool_calls"][0]["function"]["arguments"],
        "{\"a\":1}"
    );
    assert_eq!(out["choices"][0]["finish_reason"], "tool_calls");
    assert_eq!(out["usage"]["prompt_tokens"], 7);
    assert_eq!(out["usage"]["completion_tokens"], 3);
    assert_eq!(out["usage"]["total_tokens"], 10);
    // thinking 區塊**刻意丟棄**（見 anthropic_resp.rs 檔頭）
    assert!(!out.to_string().contains("祕密"), "{out}");
}

/// 串流：Anthropic 事件 → Chat chunk，含工具呼叫的片段 JSON 累積。
#[test]
fn anthropic_events_become_chat_chunks() {
    let mut st = AnthropicToChatState::default();
    let mut all: Vec<serde_json::Value> = vec![];
    for ev in [
        json!({"type": "message_start", "message": {"id": "msg_1", "model": "claude",
            "usage": {"input_tokens": 5, "output_tokens": 0}}}),
        json!({"type": "content_block_start", "index": 0,
            "content_block": {"type": "text", "text": ""}}),
        json!({"type": "content_block_delta", "index": 0,
            "delta": {"type": "text_delta", "text": "hi"}}),
        json!({"type": "content_block_stop", "index": 0}),
        // 工具：先 start（帶 id/name），再兩段片段 JSON
        json!({"type": "content_block_start", "index": 1,
            "content_block": {"type": "tool_use", "id": "t1", "name": "get"}}),
        json!({"type": "content_block_delta", "index": 1,
            "delta": {"type": "input_json_delta", "partial_json": "{\"a\":"}}),
        json!({"type": "content_block_delta", "index": 1,
            "delta": {"type": "input_json_delta", "partial_json": "1}"}}),
        json!({"type": "content_block_stop", "index": 1}),
        json!({"type": "message_delta", "delta": {"stop_reason": "tool_use"},
            "usage": {"output_tokens": 9}}),
        json!({"type": "message_stop"}),
    ] {
        all.extend(anthropic_event_to_chat_chunks(&mut st, &ev));
    }
    assert!(st.done, "message_stop 之後要標記收尾");

    let text: String = all
        .iter()
        .filter_map(|c| c["choices"][0]["delta"]["content"].as_str())
        .collect();
    assert_eq!(text, "hi");

    // 工具：第一個 chunk 帶 id/name，之後只帶 arguments 片段
    let tc: Vec<&serde_json::Value> = all
        .iter()
        .filter_map(|c| c["choices"][0]["delta"]["tool_calls"].as_array())
        .flatten()
        .collect();
    assert_eq!(tc.len(), 3, "start + 兩段片段：{tc:?}");
    assert_eq!(tc[0]["id"], "t1");
    assert_eq!(tc[0]["function"]["name"], "get");
    assert_eq!(tc[0]["index"], 0);
    let args: String = tc
        .iter()
        .filter_map(|c| c["function"]["arguments"].as_str())
        .collect();
    assert_eq!(args, "{\"a\":1}", "片段要能拼回完整 JSON");
    assert!(tc[1].get("id").is_none(), "後續片段不該重複帶 id");

    // finish_reason 與 usage 在收尾的 chunk 上
    assert!(all
        .iter()
        .any(|c| c["choices"][0]["finish_reason"] == "tool_calls"));
    assert!(all
        .iter()
        .any(|c| c["usage"]["completion_tokens"] == 9));

    // thinking_delta 不該產生任何 chunk
    let before = all.len();
    let mut st2 = AnthropicToChatState::default();
    let got = anthropic_event_to_chat_chunks(
        &mut st2,
        &json!({"type": "content_block_delta", "index": 0,
                "delta": {"type": "thinking_delta", "thinking": "祕密"}}),
    );
    assert!(got.is_empty(), "{got:?}");
    let _ = before;
}

/// Anthropic 的 error 事件 → Chat 的 error 物件。
#[test]
fn anthropic_error_becomes_chat_error() {
    let e = json!({"type": "error", "error": {"type": "overloaded_error", "message": "busy"}});
    let out = anthropic_error_to_chat(&e);
    assert_eq!(out["error"]["message"], "busy");
    assert_eq!(out["error"]["type"], "overloaded_error");
}
