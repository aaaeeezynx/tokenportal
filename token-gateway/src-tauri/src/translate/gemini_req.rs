//! OpenAI Chat Completions 請求 → Gemini `generateContent` 請求（請求方向）。
//!
//! ## Gemini 跟另外三種最不一樣的地方
//!
//! 1. **模型名在路徑裡**：`/models/{model}:generateContent`（或
//!    `:streamGenerateContent?alt=sse`）—— 所以本函式**同時回傳路徑**
//! 2. 內容是 `parts[]`，角色只有 `user` / `model`（沒有 assistant）
//! 3. 工具是 `functionDeclarations`，參數用 OpenAPI 子集
//! 4. 工具呼叫是 `parts[].functionCall`、工具結果是 `parts[].functionResponse`
//!    （**沒有 tool 角色**，結果要包成 `user` 的 part）
//! 5. system 是獨立的 `systemInstruction`
//!
//! ## 組合方式與 Anthropic 那邊一致
//!
//! - Chat 客戶端 → 本函式 → `POST /models/{m}:generateContent`
//! - Codex（Responses）→ `responses_to_openai` → 本函式（兩跳）
//!
//! 刻意重用既有的 Responses ⇄ Chat，而不是再寫一份 Responses ⇄ Gemini。

use super::*;
use serde_json::json;

/// OpenAI content（字串或 parts 陣列）→ Gemini `parts[]`。
///
/// 空白的 text part 會被丟掉：Gemini 對 `{"text":""}` 會 400。
fn content_to_parts(content: &Value) -> Vec<Value> {
    match content {
        Value::String(t) => {
            if t.is_empty() {
                vec![]
            } else {
                vec![json!({"text": t})]
            }
        }
        Value::Array(parts) => parts
            .iter()
            .filter_map(|p| match p.get("type").and_then(|t| t.as_str()) {
                Some("text") => {
                    let t = s(&p["text"]);
                    if t.is_empty() {
                        None
                    } else {
                        Some(json!({"text": t}))
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
                    Some(json!({"inlineData": {"mimeType": media, "data": data}}))
                }
                _ => None,
            })
            .collect(),
        _ => vec![],
    }
}

/// Chat `tool_choice` → Gemini `functionCallingConfig.mode`。
fn tool_choice_to_gemini(tc: &Value) -> Option<Value> {
    let (mode, names): (&str, Vec<String>) = match tc {
        Value::String(v) => match v.as_str() {
            "auto" => ("AUTO", vec![]),
            "required" => ("ANY", vec![]),
            "none" => ("NONE", vec![]),
            _ => return None,
        },
        Value::Object(o) => match o.get("type").and_then(|t| t.as_str()) {
            Some("function") => {
                let n = s(&o["function"]["name"]);
                if n.is_empty() {
                    ("ANY", vec![])
                } else {
                    ("ANY", vec![n])
                }
            }
            Some("any") => ("ANY", vec![]),
            Some("auto") => ("AUTO", vec![]),
            Some("none") => ("NONE", vec![]),
            _ => return None,
        },
        _ => return None,
    };
    let mut cfg = serde_json::Map::new();
    cfg.insert("mode".to_string(), json!(mode));
    if !names.is_empty() {
        cfg.insert("allowedFunctionNames".to_string(), json!(names));
    }
    Some(json!({"functionCallingConfig": Value::Object(cfg)}))
}

/// Chat 請求 → `(Gemini 請求體, 上游路徑)`。
///
/// 路徑依**是否串流**而不同：Gemini 的串流端點是另一個動作名，
/// 而且要 `alt=sse` 才會回 SSE（否則是一串 JSON 陣列）。
pub(crate) fn chat_to_gemini(
    req: &Value,
    model: &str,
    allowed: &[String],
) -> (Value, String) {
    let mut contents: Vec<Value> = vec![];
    let mut system_parts: Vec<String> = vec![];

    if let Some(arr) = req.get("messages").and_then(|m| m.as_array()) {
        for m in arr {
            match s(&m["role"]).as_str() {
                "system" | "developer" => {
                    let t = match &m["content"] {
                        Value::String(t) => t.clone(),
                        Value::Array(ps) => ps
                            .iter()
                            .filter(|p| p.get("type").and_then(|t| t.as_str()) == Some("text"))
                            .map(|p| s(&p["text"]))
                            .collect::<Vec<_>>()
                            .join("\n"),
                        _ => String::new(),
                    };
                    if !t.is_empty() {
                        system_parts.push(t);
                    }
                }
                "assistant" => {
                    let mut parts = content_to_parts(&m["content"]);
                    if let Some(calls) = m.get("tool_calls").and_then(|t| t.as_array()) {
                        for call in calls {
                            let f = &call["function"];
                            let args: Value = f
                                .get("arguments")
                                .and_then(|a| a.as_str())
                                .and_then(|t| serde_json::from_str(t).ok())
                                .unwrap_or_else(|| json!({}));
                            parts.push(json!({
                                "functionCall": {"name": s(&f["name"]), "args": args}
                            }));
                        }
                    }
                    if !parts.is_empty() {
                        contents.push(json!({"role": "model", "parts": parts}));
                    }
                }
                "tool" => {
                    // Gemini **沒有 tool 角色**：工具結果是 user 的一個 part，
                    // 而且 `functionResponse.response` 必須是**物件**。
                    let out = match &m["content"] {
                        Value::String(t) => t.clone(),
                        Value::Array(ps) => ps
                            .iter()
                            .filter(|p| p.get("type").and_then(|t| t.as_str()) == Some("text"))
                            .map(|p| s(&p["text"]))
                            .collect::<Vec<_>>()
                            .join(""),
                        _ => String::new(),
                    };
                    contents.push(json!({
                        "role": "user",
                        "parts": [{"functionResponse": {
                            "name": s(&m["name"]),
                            "response": {"content": out}
                        }}]
                    }));
                }
                _ => {
                    let parts = content_to_parts(&m["content"]);
                    if !parts.is_empty() {
                        contents.push(json!({"role": "user", "parts": parts}));
                    }
                }
            }
        }
    }

    let mut out = serde_json::Map::new();
    out.insert("contents".to_string(), Value::Array(contents));
    if !system_parts.is_empty() {
        out.insert(
            "systemInstruction".to_string(),
            json!({"parts": [{"text": system_parts.join("\n\n")}]}),
        );
    }

    // ---- generationConfig ----
    let mut gc = serde_json::Map::new();
    let max = req
        .get("max_tokens")
        .or_else(|| req.get("max_completion_tokens"))
        .and_then(|v| v.as_i64())
        .filter(|n| *n > 0);
    if let Some(n) = max {
        gc.insert("maxOutputTokens".to_string(), json!(n));
    }
    for (from, to) in [("temperature", "temperature"), ("top_p", "topP")] {
        if let Some(v) = req.get(from) {
            gc.insert(to.to_string(), v.clone());
        }
    }
    if let Some(stop) = req.get("stop") {
        // Chat 的 stop 可以是字串或陣列；Gemini 要陣列
        let arr = match stop {
            Value::String(t) => json!([t]),
            Value::Array(_) => stop.clone(),
            _ => json!([]),
        };
        gc.insert("stopSequences".to_string(), arr);
    }
    // 思考強度 → thinkingConfig（與 Anthropic 的映射同構）
    let effort = s(&req["reasoning_effort"]);
    if let Some(b) = thinking_budget(&effort)
        .filter(|_| allowed.iter().any(|a| a.eq_ignore_ascii_case(effort.trim())))
    {
        gc.insert("thinkingConfig".to_string(), json!({"thinkingBudget": b}));
    }
    if !gc.is_empty() {
        out.insert("generationConfig".to_string(), Value::Object(gc));
    }

    // ---- 工具 ----
    if let Some(tools) = req.get("tools").and_then(|t| t.as_array()) {
        let decls: Vec<Value> = tools
            .iter()
            .filter_map(|t| {
                let f = t.get("function").unwrap_or(t);
                let name = s(&f["name"]);
                if name.is_empty() {
                    return None;
                }
                let mut d = serde_json::Map::new();
                d.insert("name".to_string(), json!(name));
                let desc = s(&f["description"]);
                if !desc.is_empty() {
                    d.insert("description".to_string(), json!(desc));
                }
                if let Some(p) = f.get("parameters") {
                    d.insert("parameters".to_string(), p.clone());
                }
                Some(Value::Object(d))
            })
            .collect();
        if !decls.is_empty() {
            out.insert("tools".to_string(), json!([{"functionDeclarations": decls}]));
            if let Some(tc) = req.get("tool_choice").and_then(tool_choice_to_gemini) {
                out.insert("toolConfig".to_string(), tc);
            }
        }
    }

    // 路徑：模型名在裡面；串流是另一個動作名，且要 alt=sse 才回 SSE
    let stream = req.get("stream").and_then(|v| v.as_bool()).unwrap_or(false);
    let m = model.trim().trim_start_matches("models/");
    let path = if stream {
        format!("/models/{m}:streamGenerateContent?alt=sse")
    } else {
        format!("/models/{m}:generateContent")
    };
    (Value::Object(out), path)
}
