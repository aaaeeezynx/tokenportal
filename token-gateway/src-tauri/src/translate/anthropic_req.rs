//! OpenAI Chat Completions 請求 → Anthropic Messages 請求（請求方向）。
//!
//! ## 這是 2026-10-10 補上的「反向」那一半
//!
//! 既有的六個翻譯器全都是「上游＝Chat」：請求翻成 Chat 送出去，回應從 Chat
//! 翻回客戶端格式。要讓 OpenAI 格式的客戶端（Codex、各種 Chat 工具）用得了
//! Anthropic 上游（例如 opencode-go 的 `claude-haiku-5-5`），缺的正是反過來
//! 的這一半 —— 本檔（請求）與 `anthropic_resp.rs`（回應）。
//!
//! ## 兩個組合路徑
//!
//! - Chat 客戶端 → 本函式 → `POST /messages`
//! - Codex（Responses）→ `responses_to_openai` → 本函式 → `POST /messages`
//!
//! 第二條刻意**重用**既有的 Responses ⇄ Chat 那一段，而不是再寫一份
//! Responses ⇄ Anthropic 的直接翻譯。

use super::*;
use serde_json::json;

/// Anthropic 的 `max_tokens` 是**必填**（Chat 與 Responses 都不是）。
/// 客戶端沒帶時用這個值。
const DEFAULT_MAX_TOKENS: i64 = 8_192;

/// `budget_tokens` 必須**嚴格小於** `max_tokens`；留這麼多給正文。
const MAX_TOKENS_HEADROOM: i64 = 1_024;

/// OpenAI content（字串或 parts 陣列）→ Anthropic content blocks。
///
/// 空白的 text 區塊會被丟掉：Anthropic 對 `{"type":"text","text":""}` 會 400。
fn content_to_anthropic(content: &Value) -> Value {
    match content {
        Value::String(t) => {
            if t.is_empty() {
                json!([])
            } else {
                json!([{"type": "text", "text": t}])
            }
        }
        Value::Array(parts) => {
            let blocks: Vec<Value> = parts
                .iter()
                .filter_map(|p| match p.get("type").and_then(|t| t.as_str()) {
                    Some("text") => {
                        let t = s(&p["text"]);
                        if t.is_empty() {
                            None
                        } else {
                            Some(json!({"type": "text", "text": t}))
                        }
                    }
                    Some("image_url") => {
                        // data:<media_type>;base64,<data>
                        let url = s(&p["image_url"]["url"]);
                        let (head, data) = url.split_once(";base64,")?;
                        let media = head.strip_prefix("data:").unwrap_or(head);
                        if data.is_empty() {
                            return None;
                        }
                        Some(json!({
                            "type": "image",
                            "source": {"type": "base64", "media_type": media, "data": data}
                        }))
                    }
                    _ => None,
                })
                .collect();
            Value::Array(blocks)
        }
        _ => json!([]),
    }
}

/// `tool_choice`：Chat → Anthropic。
fn tool_choice_to_anthropic(tc: &Value) -> Option<Value> {
    match tc {
        Value::String(v) => match v.as_str() {
            "auto" => Some(json!({"type": "auto"})),
            "required" => Some(json!({"type": "any"})),
            "none" => Some(json!({"type": "none"})),
            _ => None,
        },
        Value::Object(o) => match o.get("type").and_then(|t| t.as_str()) {
            Some("function") => {
                let name = s(&o["function"]["name"]);
                if name.is_empty() {
                    None
                } else {
                    Some(json!({"type": "tool", "name": name}))
                }
            }
            Some("any") => Some(json!({"type": "any"})),
            Some("auto") => Some(json!({"type": "auto"})),
            Some("none") => Some(json!({"type": "none"})),
            _ => None,
        },
        _ => None,
    }
}

/// Chat 請求 → Anthropic 請求。
///
/// `model` 由調用方先映射好傳入；`allowed` 是這個模型允許的思考強度
/// （見 [`crate::models::allowed_efforts`]）——**只有被允許的檔位才會開延伸思考**，
/// 與 Chat 路徑同一道閘門，避免使用者設了上游不認的值。
pub(crate) fn chat_to_anthropic(req: &Value, model: &str, allowed: &[String]) -> Value {
    let mut system_parts: Vec<String> = vec![];
    let mut messages: Vec<Value> = vec![];

    if let Some(arr) = req.get("messages").and_then(|m| m.as_array()) {
        let mut i = 0usize;
        while i < arr.len() {
            let m = &arr[i];
            match s(&m["role"]).as_str() {
                "system" | "developer" => {
                    let t = match &m["content"] {
                        Value::String(t) => t.clone(),
                        Value::Array(ps) => ps
                            .iter()
                            .filter(|p| {
                                p.get("type").and_then(|t| t.as_str()) == Some("text")
                            })
                            .map(|p| s(&p["text"]))
                            .collect::<Vec<_>>()
                            .join("\n"),
                        _ => String::new(),
                    };
                    if !t.is_empty() {
                        system_parts.push(t);
                    }
                    i += 1;
                }
                "tool" => {
                    // 連續的 tool 結果**併成同一個 user 訊息**：Anthropic 的正規
                    // 形狀是一次回一批 tool_result，拆成多個 user 訊息雖然多半也
                    // 收，但對某些上游會被判成「tool_result 不在 user 訊息裡」。
                    let mut blocks: Vec<Value> = vec![];
                    while i < arr.len() && s(&arr[i]["role"]) == "tool" {
                        let tm = &arr[i];
                        let out = match &tm["content"] {
                            Value::String(t) => t.clone(),
                            Value::Array(ps) => ps
                                .iter()
                                .filter(|p| {
                                    p.get("type").and_then(|t| t.as_str()) == Some("text")
                                })
                                .map(|p| s(&p["text"]))
                                .collect::<Vec<_>>()
                                .join(""),
                            _ => String::new(),
                        };
                        blocks.push(json!({
                            "type": "tool_result",
                            "tool_use_id": s(&tm["tool_call_id"]),
                            "content": out,
                        }));
                        i += 1;
                    }
                    messages.push(json!({"role": "user", "content": blocks}));
                }
                "assistant" => {
                    let mut blocks: Vec<Value> = vec![];
                    let c = content_to_anthropic(&m["content"]);
                    if let Some(a) = c.as_array() {
                        blocks.extend(a.iter().cloned());
                    }
                    if let Some(calls) = m.get("tool_calls").and_then(|t| t.as_array()) {
                        for call in calls {
                            let f = &call["function"];
                            let input: Value = f
                                .get("arguments")
                                .and_then(|a| a.as_str())
                                .and_then(|t| serde_json::from_str(t).ok())
                                .unwrap_or_else(|| json!({}));
                            blocks.push(json!({
                                "type": "tool_use",
                                "id": s(&call["id"]),
                                "name": s(&f["name"]),
                                "input": input,
                            }));
                        }
                    }
                    // Anthropic 不接受 content 為空陣列的 assistant 訊息
                    if !blocks.is_empty() {
                        messages.push(json!({"role": "assistant", "content": blocks}));
                    }
                    i += 1;
                }
                _ => {
                    let c = content_to_anthropic(&m["content"]);
                    let empty = c.as_array().map(|a| a.is_empty()).unwrap_or(false);
                    if !empty {
                        messages.push(json!({"role": "user", "content": c}));
                    }
                    i += 1;
                }
            }
        }
    }

    let mut out = serde_json::Map::new();
    out.insert("model".to_string(), Value::String(model.to_string()));
    out.insert("messages".to_string(), Value::Array(messages));
    if !system_parts.is_empty() {
        out.insert(
            "system".to_string(),
            Value::String(system_parts.join("\n\n")),
        );
    }

    // ---- max_tokens（必填）＋ 延伸思考 ----
    let requested = req
        .get("max_tokens")
        .or_else(|| req.get("max_completion_tokens"))
        .and_then(|v| v.as_i64())
        .filter(|n| *n > 0)
        .unwrap_or(DEFAULT_MAX_TOKENS);
    let effort = s(&req["reasoning_effort"]);
    let budget = thinking_budget(&effort).filter(|_| {
        allowed.iter().any(|a| a.eq_ignore_ascii_case(effort.trim()))
    });
    let mut max_tokens = requested;
    if let Some(b) = budget {
        // Anthropic 要求 `budget_tokens < max_tokens`。客戶端給的上限若裝不下
        // 思考預算，**往上調 max_tokens**（它只是上限，模型該停還是會停），
        // 而不是把預算壓小 —— 壓小就等於使用者選的檔位沒有意義。
        max_tokens = max_tokens.max(b + MAX_TOKENS_HEADROOM);
        out.insert(
            "thinking".to_string(),
            json!({"type": "enabled", "budget_tokens": b}),
        );
    }
    out.insert("max_tokens".to_string(), json!(max_tokens));

    // ---- 取樣參數 ----
    // Anthropic：開啟延伸思考時**不能**同時改 temperature／top_p／top_k。
    // 與其讓上游回 400，不如丟掉它們（思考優先，因為那是使用者明確選的）。
    if budget.is_none() {
        for a in ["temperature", "top_p"] {
            if let Some(v) = req.get(a) {
                out.insert(a.to_string(), v.clone());
            }
        }
    }
    if let Some(stop) = req.get("stop") {
        out.insert("stop_sequences".to_string(), stop.clone());
    }
    if let Some(st) = req.get("stream") {
        out.insert("stream".to_string(), st.clone());
    }

    // ---- 工具 ----
    if let Some(tools) = req.get("tools").and_then(|t| t.as_array()) {
        let mapped: Vec<Value> = tools
            .iter()
            .filter_map(|t| {
                // Chat 的 tools 是 {type:"function", function:{...}}
                let f = t.get("function").unwrap_or(t);
                let name = s(&f["name"]);
                if name.is_empty() {
                    return None;
                }
                Some(json!({
                    "name": name,
                    "description": s(&f["description"]),
                    "input_schema": f
                        .get("parameters")
                        .cloned()
                        .unwrap_or_else(|| json!({"type": "object"})),
                }))
            })
            .collect();
        if !mapped.is_empty() {
            out.insert("tools".to_string(), Value::Array(mapped));
            if let Some(tc) = req.get("tool_choice").and_then(tool_choice_to_anthropic) {
                out.insert("tool_choice".to_string(), tc);
            }
        }
    }
    Value::Object(out)
}
