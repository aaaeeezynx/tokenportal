//! Thinking Signature 整流器（對標 CC Switch 的 thinking_rectifier）。
//!
//! ## 解決什麼問題
//!
//! **多輪對話的第二次請求**：上游的回應帶 thinking block（Anthropic）或
//! `thoughtSignature`（Gemini），客戶端把歷史原樣送回來時，簽章可能
//! 過期／被改／不被這個渠道接受 → 上游 400，而且**每次續聊都失敗**。
//! 這正是使用者回報的「無法繼續對話」。
//!
//! ## 做什麼
//!
//! 上游 400 的錯誤訊息符合 7 種簽章錯誤模式之一時，對**送出去的 body**
//! 做最小侵入修復後重試一次：
//!
//! - 移除 `messages[*].content` 裡的 `thinking`／`redacted_thinking` block
//! - 移除非 thinking block 上殘留的 `signature` 欄位
//! - 特定條件下移除頂層 `thinking`（開啟中 ＋ 最後一條 assistant 以 tool_use
//!   開頭卻沒有 thinking 前綴 —— Anthropic 的硬性要求）
//! - Gemini：移除 `contents[].parts[]` 上的 `thoughtSignature`
//!
//! ## 為什麼不記住（與 `provider_stripped_fields` 不同）
//!
//! 剝欄位是「這個來源永遠不收這個欄位」—— 記住一次就永久有效。
//! 簽章錯誤是「**這一次**的簽章壞了」—— 下一輪的新簽章可能是好的。
//! 記住會讓之後所有請求都盲目剝簽章，反而把有用的狀態丟掉。
//! CC Switch 也不記住（每次錯誤都現場修）。
//!
//! ## 為什麼觸發條件裡有一條很寬的「invalid request」
//!
//! 上游對簽章問題的措辭五花八門（各家渠道轉述 Google／Anthropic 的原文），
//! 最後一條是兜底：修復動作本身是安全的（移除的是**歷史**思考塊，不影響
//! 本輪生成），代價只是多一次上游呼叫；若修復後仍失敗，原本的錯誤會原樣
//! 透出（見 retry.rs 的既有流程），不會吞掉真正的錯誤。

use serde_json::Value;

/// 上游 400 的錯誤訊息是否為「簽章類」錯誤。
///
/// 7 個場景的順序：具體的在前，兜底的在最後。
pub(crate) fn is_signature_error(body: &str) -> bool {
    let lower = body.to_lowercase();

    // 1: thinking block 的簽章無效（Anthropic 原文）
    if lower.contains("invalid")
        && lower.contains("signature")
        && lower.contains("thinking")
        && lower.contains("block")
    {
        return true;
    }
    // 1b: Gemini／第三方「Thought signature is not valid」
    if lower.contains("thought signature")
        && (lower.contains("not valid") || lower.contains("invalid"))
    {
        return true;
    }
    // 2: assistant 必須以 thinking block 開頭
    if lower.contains("must start with a thinking block") {
        return true;
    }
    // 3: 預期 thinking 卻遇到 tool_use（要求明確含 tool_use，避免過寬）
    if lower.contains("expected")
        && (lower.contains("thinking") || lower.contains("redacted_thinking"))
        && lower.contains("found")
        && lower.contains("tool_use")
    {
        return true;
    }
    // 4: signature 欄位必填但缺失
    if lower.contains("signature") && lower.contains("field required") {
        return true;
    }
    // 5: signature 欄位不被接受（第三方渠道）
    if lower.contains("signature") && lower.contains("extra inputs are not permitted") {
        return true;
    }
    // 6: thinking/redacted_thinking 被修改
    if (lower.contains("thinking") || lower.contains("redacted_thinking"))
        && lower.contains("cannot be modified")
    {
        return true;
    }
    // 7: 兜底（與 CCH 對齊）
    lower.contains("非法請求")
        || lower.contains("illegal request")
        || lower.contains("invalid request")
}

/// 修復統計。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct RectifyResult {
    pub applied: bool,
    pub removed_thinking: usize,
    pub removed_redacted: usize,
    pub removed_signatures: usize,
    pub removed_thought_signatures: usize,
}

impl RectifyResult {
    fn any(&self) -> bool {
        self.applied
    }
}

/// 對送出去的 body 做整流。回 `None` ＝ 沒有需要修的（原樣重試沒意義）。
///
/// 依形狀分派：有 `messages` 走 Anthropic 式；有 `contents` 走 Gemini 式。
pub(crate) fn rectify(raw: &[u8]) -> Option<Vec<u8>> {
    let Ok(mut v) = serde_json::from_slice::<Value>(raw) else {
        return None;
    };
    let mut r = RectifyResult::default();
    if v.get("messages").is_some() {
        rectify_anthropic(&mut v, &mut r);
    } else if v.get("contents").is_some() {
        rectify_gemini(&mut v, &mut r);
    } else {
        return None;
    }
    if !r.any() {
        return None;
    }
    serde_json::to_vec(&v).ok()
}

/// Anthropic Messages 請求的最小侵入整流。
fn rectify_anthropic(v: &mut Value, r: &mut RectifyResult) {
    let Some(messages) = v.get_mut("messages").and_then(|m| m.as_array_mut()) else {
        return;
    };
    for msg in messages.iter_mut() {
        let Some(content) = msg.get_mut("content").and_then(|c| c.as_array_mut()) else {
            continue;
        };
        let mut keep: Vec<Value> = Vec::with_capacity(content.len());
        let mut changed = false;
        for block in content.iter() {
            let ty = block.get("type").and_then(|t| t.as_str());
            match ty {
                Some("thinking") => {
                    r.removed_thinking += 1;
                    changed = true;
                }
                Some("redacted_thinking") => {
                    r.removed_redacted += 1;
                    changed = true;
                }
                _ => {
                    // 非 thinking block 上殘留的 signature 一併清掉
                    if block.get("signature").is_some() {
                        if let Some(obj) = block.as_object() {
                            let mut c = obj.clone();
                            c.remove("signature");
                            r.removed_signatures += 1;
                            changed = true;
                            keep.push(Value::Object(c));
                            continue;
                        }
                    }
                    keep.push(block.clone());
                }
            }
        }
        if changed {
            r.applied = true;
            *content = keep;
        }
    }
    if should_drop_top_level_thinking(v) {
        if let Some(obj) = v.as_object_mut() {
            obj.remove("thinking");
            r.applied = true;
        }
    }
}

/// 頂層 `thinking` 何時該拿掉：開啟中 ＋ 最後一條 assistant 以 tool_use 開頭
/// 卻沒有 thinking 前綴（Anthropic 要求 tool-use 鏈的 assistant 必須以
/// thinking 開頭；修不掉前綴就只能整個關掉 thinking）。
fn should_drop_top_level_thinking(v: &Value) -> bool {
    let enabled = v
        .get("thinking")
        .and_then(|t| t.get("type"))
        .and_then(|t| t.as_str())
        == Some("enabled");
    if !enabled {
        return false;
    }
    let Some(last) = v
        .get("messages")
        .and_then(|m| m.as_array())
        .and_then(|m| {
            m.iter()
                .rev()
                .find(|m| m.get("role").and_then(|r| r.as_str()) == Some("assistant"))
        })
    else {
        return false;
    };
    let Some(content) = last.get("content").and_then(|c| c.as_array()) else {
        return false;
    };
    if content.is_empty() {
        return false;
    }
    let first = content[0].get("type").and_then(|t| t.as_str());
    let missing_prefix = first != Some("thinking") && first != Some("redacted_thinking");
    let has_tool_use = content
        .iter()
        .any(|b| b.get("type").and_then(|t| t.as_str()) == Some("tool_use"));
    missing_prefix && has_tool_use
}

/// Gemini generateContent 請求：移除 parts 上的 `thoughtSignature`。
fn rectify_gemini(v: &mut Value, r: &mut RectifyResult) {
    let Some(contents) = v.get_mut("contents").and_then(|c| c.as_array_mut()) else {
        return;
    };
    for content in contents.iter_mut() {
        let Some(parts) = content.get_mut("parts").and_then(|p| p.as_array_mut()) else {
            continue;
        };
        for part in parts.iter_mut() {
            if part.get("thoughtSignature").is_some() {
                if let Some(obj) = part.as_object_mut() {
                    obj.remove("thoughtSignature");
                    r.removed_thought_signatures += 1;
                    r.applied = true;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trig(msg: &str) -> bool {
        is_signature_error(msg)
    }

    /// 7 個場景都要命中（含大小寫差異）。
    #[test]
    fn all_documented_error_shapes_trigger() {
        assert!(trig("Invalid 'signature' in 'thinking' block"));
        assert!(trig("Unable to submit request because Thought signature is not valid"));
        assert!(trig("messages: assistant message must start with a thinking block"));
        assert!(trig("Expected `thinking` or `redacted_thinking`, but found `tool_use`"));
        assert!(trig("thinking.signature: Field required"));
        assert!(trig("messages.1.signature: Extra inputs are not permitted"));
        assert!(trig("thinking or redacted_thinking blocks cannot be modified"));
        // 兜底
        assert!(trig("invalid request"));
        assert!(trig("非法請求"));
        // 大小寫不敏感
        assert!(trig("INVALID 'SIGNATURE' IN 'THINKING' BLOCK"));
    }

    /// 無關的錯誤不能誤觸發（否則會盲目剝思考塊）。
    #[test]
    fn unrelated_errors_do_not_trigger() {
        assert!(!trig("model not found"));
        assert!(!trig("rate limit exceeded"));
        assert!(!trig("unauthorized"));
        assert!(!trig("quota exceeded for project"));
        assert!(!trig(""));
    }

    const A_BODY: &str = r#"{
      "model": "x", "thinking": {"type": "enabled", "budget_tokens": 1024},
      "messages": [
        {"role": "user", "content": "hi"},
        {"role": "assistant", "content": [
          {"type": "thinking", "thinking": "...", "signature": "SIG1"},
          {"type": "tool_use", "id": "t1", "name": "f", "input": {}}
        ]},
        {"role": "user", "content": [{"type": "tool_result", "tool_use_id": "t1"},
          {"type": "text", "text": "go", "signature": "STRAY"}]}
      ]
    }"#;

    #[test]
    fn strips_thinking_blocks_and_stray_signatures() {
        let out = rectify(A_BODY.as_bytes()).expect("有東西要修");
        let v: Value = serde_json::from_slice(&out).unwrap();
        let msgs = v["messages"].as_array().unwrap();
        // assistant：thinking block 被移除，剩 tool_use
        let a = &msgs[1]["content"].as_array().unwrap();
        assert_eq!(a.len(), 1);
        assert_eq!(a[0]["type"], "tool_use");
        // user：text block 上的殘留 signature 被清掉
        let u = &msgs[2]["content"].as_array().unwrap();
        assert!(u.iter().all(|b| b.get("signature").is_none()));
        assert_eq!(u[1]["type"], "text", "本體要保留");
    }

    /// 修復後仍保留其他欄位（model／system 等不能掉）。
    #[test]
    fn preserves_unrelated_fields() {
        let out = rectify(A_BODY.as_bytes()).unwrap();
        let v: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(v["model"], "x");
        // 頂層 thinking 在此例應被移除（assistant 以 tool_use 開頭）
        assert!(v.get("thinking").is_none(), "tool_use 開頭卻無 thinking 前綴，頂層 thinking 應移除");
    }

    /// 頂層 thinking 的移除條件：必須「有 tool_use」才拿掉。
    ///
    /// 注意順序（與 cc-switch 一致）：先移除歷史 thinking block、再檢查頂層，
    /// 所以「assistant 原本以 thinking 開頭」的 case 在檢查時已經變成
    /// tool_use 開頭 —— 頂層會被移除。這條測試釘的是「沒有 tool_use 就不動」。
    #[test]
    fn keeps_top_level_thinking_without_tool_use() {
        let body = r#"{
          "thinking": {"type": "enabled"},
          "messages": [
            {"role": "user", "content": "hi"},
            {"role": "assistant", "content": [
              {"type": "text", "text": "ok", "signature": "S"}
            ]}
          ]
        }"#;
        let out = rectify(body.as_bytes()).unwrap();
        let v: Value = serde_json::from_slice(&out).unwrap();
        // 殘留 signature 清掉了，但頂層 thinking 不該被動（沒有 tool_use）
        assert!(v["messages"][1]["content"][0].get("signature").is_none());
        assert_eq!(v["thinking"]["type"], "enabled");
    }

    /// 反過來：tool_use 開頭卻無 thinking 前綴 → 頂層 thinking 一併移除
    ///（Anthropic 硬性要求 tool-use 鏈的 assistant 以 thinking 開頭）。
    #[test]
    fn drops_top_level_thinking_for_tool_use_chain() {
        let out = rectify(A_BODY.as_bytes()).unwrap();
        let v: Value = serde_json::from_slice(&out).unwrap();
        // A_BODY 的頂層有 thinking（enabled），assistant 移除 thinking 後以
        // tool_use 開頭 → 頂層應一併移除
        assert!(v.get("thinking").is_none(), "頂層應移除：{:?}", v.get("thinking"));
    }

    /// 沒有 thinking／signature 的乾淨 body → None（不需要重試）。
    #[test]
    fn clean_body_returns_none() {
        let clean = r#"{"model":"x","messages":[{"role":"user","content":"hi"}]}"#;
        assert!(rectify(clean.as_bytes()).is_none());
        assert!(rectify(b"not json").is_none());
        assert!(rectify(b"[]").is_none());
    }

    /// Gemini：parts 上的 thoughtSignature 被清掉，其他欄位保留。
    #[test]
    fn gemini_thought_signature_is_stripped() {
        let body = r#"{
          "contents": [
            {"role": "user", "parts": [{"text": "hi"}]},
            {"role": "model", "parts": [
              {"text": "ok", "thoughtSignature": "TSIG"},
              {"functionCall": {"name": "f"}}
            ]}
          ]
        }"#;
        let out = rectify(body.as_bytes()).expect("有 thoughtSignature 要清");
        let v: Value = serde_json::from_slice(&out).unwrap();
        let parts = v["contents"][1]["parts"].as_array().unwrap();
        assert!(parts.iter().all(|p| p.get("thoughtSignature").is_none()));
        assert_eq!(parts[1]["functionCall"]["name"], "f", "本體要保留");
    }

    /// 修復不會把空 messages 弄壞，也不會動到非物件的 content。
    #[test]
    fn degenerate_shapes_are_safe() {
        let s = r#"{"messages":[{"role":"user","content":"plain string"}]}"#;
        assert!(rectify(s.as_bytes()).is_none(), "字串 content 無 thinking 可修");
        let s2 = r#"{"contents":[{"parts":[]}]}"#;
        assert!(rectify(s2.as_bytes()).is_none());
    }
}
