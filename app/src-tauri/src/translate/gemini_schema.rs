//! Gemini 的 JSON Schema 清洗（`functionDeclarations.parameters`）。
//!
//! 為什麼要單獨一個模組：`gemini_req.rs` 已經接近專案的 400 行上限，而這一段
//! 是完全自成一體的純函式（輸入 schema、輸出 schema），搬出來最好讀。

use serde_json::{json, Value};

/// Gemini 的 `functionDeclarations.parameters` 只吃 **OpenAPI 子集**，
/// 而且它是**嚴格解析**：遇到不認得的關鍵字就整顆請求 400，不是忽略。
///
/// 2026-10-12 真機上的實例（Codex → Gemini）：
///
/// ```text
/// [400] Invalid JSON payload received. Unknown name "additionalProperties"
///       at 'tools[0].functionDeclarations[0].parameters'
/// ```
///
/// Codex／OpenAI 送的是完整 JSON Schema（`additionalProperties`、`$schema`、
/// `exclusiveMinimum`、`type: ["string","null"]` …），原樣透傳必爆。
///
/// 所以這裡是**白名單**而不是黑名單 —— 黑名單永遠追不上對方送來的關鍵字，
/// 而 Gemini 對未知字是硬錯誤。
pub(crate) fn sanitize(v: &Value) -> Value {
    /// Gemini Schema 認得的鍵（其餘一律丟掉）。
    const KEEP: [&str; 20] = [
        "type",
        "format",
        "title",
        "description",
        "nullable",
        "enum",
        "items",
        "properties",
        "required",
        "anyOf",
        "minimum",
        "maximum",
        "minItems",
        "maxItems",
        "minLength",
        "maxLength",
        "pattern",
        "default",
        "example",
        "propertyOrdering",
    ];
    match v {
        Value::Object(o) => {
            let mut out = serde_json::Map::new();
            for (k, val) in o {
                // `properties` 的**鍵是使用者自訂的屬性名**，不能套白名單
                //（否則一個叫 `type` 的參數會被吃掉），只清洗它的值。
                if k == "properties" {
                    if let Some(map) = val.as_object() {
                        let mut props = serde_json::Map::new();
                        for (name, schema) in map {
                            props.insert(name.clone(), sanitize(schema));
                        }
                        out.insert("properties".to_string(), Value::Object(props));
                    }
                    continue;
                }
                // `const` → `enum: [x]`（Gemini 沒有 const）
                if k == "const" {
                    out.insert("enum".to_string(), json!([val]));
                    continue;
                }
                // `oneOf` → `anyOf`（Gemini 只認 anyOf；語意略鬆但可用）
                if k == "oneOf" {
                    out.insert("anyOf".to_string(), sanitize(val));
                    continue;
                }
                // `type: ["string","null"]` → `type: "string"` ＋ `nullable: true`
                if k == "type" {
                    if let Some(arr) = val.as_array() {
                        let mut ty: Option<&str> = None;
                        let mut nullable = false;
                        for x in arr {
                            match x.as_str() {
                                Some("null") => nullable = true,
                                Some(t) => ty = Some(t),
                                None => {}
                            }
                        }
                        if let Some(t) = ty {
                            out.insert("type".to_string(), json!(t));
                        }
                        if nullable {
                            out.insert("nullable".to_string(), json!(true));
                        }
                        continue;
                    }
                }
                if KEEP.contains(&k.as_str()) {
                    out.insert(k.clone(), sanitize(val));
                }
            }
            Value::Object(out)
        }
        Value::Array(a) => Value::Array(a.iter().map(sanitize).collect()),
        _ => v.clone(),
    }
}
