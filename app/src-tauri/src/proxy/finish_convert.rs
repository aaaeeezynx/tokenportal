//! 非串流的回應形狀轉換（`finish.rs` 的「整包讀完再轉」那一段）。
//!
//! ## 為什麼抽出來
//!
//! 這段是**純函式**：給上游 body ＋ `TransKind` ＋ 模型名，回傳要送給客戶端的
//! body。它原本埋在 `finish_response` 裡，跟串流分支、記帳、標頭轉發混在一起
//! —— 那種地方沒辦法測，只能靠實機。
//!
//! 抽出來之後 `finish.rs` 回到 400 行以內，而這裡可以逐格驗證「兩跳的第二段」
//! 到底有沒有接對（`AnthropicToGemini` 就是這種兩跳，最容易接錯）。
//!
//! ## 兩跳的規則（與 `proxy/matrix.rs` 的矩陣一一對應）
//!
//! 第一段：**上游格式 → Chat**
//! - Anthropic 上游（`*ToAnthropic`）→ `anthropic_to_chat_resp`
//! - Gemini 上游（`*ToGemini`）→ `gemini_to_chat_resp`
//! - 其餘（本來就是 Chat 上游）→ 原樣
//!
//! 第二段：**Chat → 客戶端格式**
//! - Responses 客戶端（`ResponsesTo*`）→ `openai_to_responses`
//! - Chat 客戶端（`ChatTo*`）→ 原樣
//! - 其餘（Anthropic 客戶端）→ `openai_to_anthropic`

use super::matrix::TransKind;

/// 這個 `TransKind` 的上游是不是 Gemini。
fn upstream_is_gemini(kind: TransKind) -> bool {
    matches!(
        kind,
        TransKind::ChatToGemini
            | TransKind::ResponsesToGemini
            | TransKind::AnthropicToGemini
    )
}

/// 這個 `TransKind` 的上游是不是 Anthropic。
fn upstream_is_anthropic(kind: TransKind) -> bool {
    matches!(
        kind,
        TransKind::ChatToAnthropic | TransKind::ResponsesToAnthropic
    )
}

/// 這個 `TransKind` 的客戶端是不是 Responses（Codex）。
fn client_is_responses(kind: TransKind) -> bool {
    matches!(
        kind,
        TransKind::ResponsesToChat
            | TransKind::ResponsesToAnthropic
            | TransKind::ResponsesToGemini
    )
}

/// 這個 `TransKind` 的客戶端是不是 Chat。
fn client_is_chat(kind: TransKind) -> bool {
    matches!(
        kind,
        TransKind::ChatToAnthropic | TransKind::ChatToGemini
    )
}

/// 把上游 body 轉成客戶端要的形狀。`None` = 不需要轉（透傳或形狀本來就對）。
///
/// 只有**成功狀態**才呼叫（錯誤回包必須原樣透出，客戶端靠狀態碼與 `error` 讀訊息）。
pub(super) fn convert_body(
    kind: TransKind,
    upstream: &serde_json::Value,
    model_raw: &str,
    custom_tools: &[String],
) -> Option<serde_json::Value> {
    // ── 第一段：上游 → Chat ──
    let chat = if upstream_is_anthropic(kind) {
        crate::translate::anthropic_to_chat_resp(upstream, model_raw)
    } else if upstream_is_gemini(kind) {
        crate::translate::gemini_to_chat_resp(upstream, model_raw)
    } else {
        upstream.clone()
    };

    // ── 第二段：Chat → 客戶端 ──
    Some(if client_is_responses(kind) {
        crate::translate::openai_to_responses(&chat, model_raw, custom_tools)
    } else if client_is_chat(kind) {
        chat
    } else {
        crate::translate::openai_to_anthropic(&chat, model_raw)
    })
}

#[cfg(test)]
pub(super) fn describe(kind: TransKind) -> String {
    format!(
        "{:?}（上游 gemini={} anthropic={}；客戶端 responses={} chat={}）",
        kind,
        upstream_is_gemini(kind),
        upstream_is_anthropic(kind),
        client_is_responses(kind),
        client_is_chat(kind)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn gemini_resp() -> serde_json::Value {
        json!({
            "candidates": [{
                "finishReason": "STOP",
                "content": {"role": "model", "parts": [{"text": "好"}]}
            }],
            "usageMetadata": {"promptTokenCount": 5, "candidatesTokenCount": 2}
        })
    }

    /// 四個 `*ToGemini` 都必須被認成「上游是 Gemini」—— 這是兩跳第一段的依據。
    #[test]
    fn every_gemini_direction_is_recognised() {
        for k in [
            TransKind::ChatToGemini,
            TransKind::ResponsesToGemini,
            TransKind::AnthropicToGemini,
        ] {
            assert!(upstream_is_gemini(k), "{} 應認成 Gemini 上游", describe(k));
            assert!(!upstream_is_anthropic(k));
        }
        // 反向（Gemini 當客戶端）不算：那是 `TransKind::None` 透傳
        assert!(!upstream_is_gemini(TransKind::None));
    }

    /// 第二段的客戶端判定：Responses／Chat／Anthropic 三選一，互斥。
    #[test]
    fn client_side_is_mutually_exclusive() {
        for k in [
            TransKind::None,
            TransKind::AnthropicToChat,
            TransKind::ResponsesToChat,
            TransKind::ChatToAnthropic,
            TransKind::ResponsesToAnthropic,
            TransKind::ChatToGemini,
            TransKind::ResponsesToGemini,
            TransKind::AnthropicToGemini,
        ] {
            let r = client_is_responses(k);
            let c = client_is_chat(k);
            assert!(!(r && c), "{} 同時被認成 Responses 與 Chat", describe(k));
        }
    }

    /// `AnthropicToGemini`（2026-10-20 新增）＝ Gemini 上游 ＋ Anthropic 客戶端。
    #[test]
    fn anthropic_to_gemini_is_two_hop_to_anthropic() {
        assert!(upstream_is_gemini(TransKind::AnthropicToGemini));
        assert!(!client_is_responses(TransKind::AnthropicToGemini));
        assert!(!client_is_chat(TransKind::AnthropicToGemini));
        // 因此第二段會走 `openai_to_anthropic`
        let out = convert_body(
            TransKind::AnthropicToGemini,
            &gemini_resp(),
            "claude-sonnet-5",
            &[],
        )
        .expect("一定要轉");
        // Anthropic 形狀：type=message、有 content 陣列
        assert_eq!(out["type"], "message");
        assert!(out["content"].is_array(), "應是 Anthropic content 陣列");
        assert_eq!(out["model"], "claude-sonnet-5");
    }

    /// `ChatToGemini`（Chat 客戶端）第二段不翻譯，直接回 Chat 形狀。
    #[test]
    fn chat_client_stops_at_chat() {
        let out = convert_body(TransKind::ChatToGemini, &gemini_resp(), "m", &[])
            .expect("要轉");
        assert!(
            out.get("choices").is_some(),
            "Chat 客戶端應拿到 choices 陣列：{out}"
        );
        assert!(out.get("content").is_none(), "不該是 Anthropic 形狀");
    }

}
