//! 逐協議的路徑前綴（`providers.protocol_prefixes`）。
//!
//! ## 為什麼需要這一層
//!
//! 有些上游把「協議」做在**路徑前綴**裡，而不是靠端點字尾區分。實測 OpenCode Go
//! 的文件端點（同一個 base）：
//!
//! ```text
//! {base}/openai/v1/chat/completions
//! {base}/openai/v1/responses
//! {base}/anthropic/v1/messages
//! {base}/google/v1beta/models/{model}:generateContent
//! ```
//!
//! 用單一 `base_url` 加協議字尾描述不了，使用者只能為每種協議各開一個來源
//!（金鑰重複、用量／健康度統計被切碎）。有了這層前綴，**一個來源能服務全部四種**。
//!
//! ## 語意
//!
//! 前綴接在 `base_url` **之後**、協議字尾**之前**：
//!
//! ```text
//! 最終 URL = {base_url}{prefix}{協議字尾}
//! chat      → {base}{/openai/v1}    + /chat/completions
//! responses → {base}{/openai/v1}    + /responses
//! anthropic → {base}{/anthropic/v1} + /messages
//! gemini    → {base}{/google/v1beta} + /models/{m}:generateContent
//! ```
//!
//! 空字串 = 沒有前綴（既有行為，向後相容）。整份 JSON 為空也一樣。

use serde::{Deserialize, Serialize};

/// 一份來源的逐協議前綴。缺的鍵就是「沒有前綴」。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProtocolPrefixes {
    // 短名（`chat`／`responses`）也收 —— 那是探測器與路由內部用的寫法，
    // 手寫 JSON 時兩種都通。
    #[serde(default, rename = "openai-chat", alias = "chat")]
    pub openai_chat: String,
    #[serde(default, rename = "openai-responses", alias = "responses")]
    pub openai_responses: String,
    #[serde(default)]
    pub anthropic: String,
    #[serde(default)]
    pub gemini: String,
}

impl ProtocolPrefixes {
    /// 解析資料庫裡的 JSON。壞掉或空字串都回預設（**絕不讓設定錯誤擋住請求**）。
    pub(crate) fn parse(raw: &str) -> Self {
        let t = raw.trim();
        if t.is_empty() {
            return Self::default();
        }
        serde_json::from_str(t).unwrap_or_default()
    }

    /// 這個協議的前綴（會正規化簡寫：`chat` → `openai-chat`）。
    ///
    /// 回傳的片段**保證**以 `/` 開頭且不以 `/` 結尾（空字串代表沒有前綴），
    /// 呼叫端可以直接 `format!("{base}{prefix}{suffix}")`。
    pub(crate) fn for_protocol(&self, protocol: &str) -> String {
        let raw = match protocol.trim().to_ascii_lowercase().as_str() {
            "openai-chat" | "chat" => &self.openai_chat,
            "openai-responses" | "responses" => &self.openai_responses,
            "anthropic" => &self.anthropic,
            "gemini" => &self.gemini,
            _ => return String::new(),
        };
        normalize_prefix(raw)
    }
}

/// 把使用者填的前綴正規化成 `"/a/b"` 這種形狀（空 = 沒有）。
fn normalize_prefix(raw: &str) -> String {
    let t = raw.trim();
    if t.is_empty() {
        return String::new();
    }
    let t = t.trim_end_matches('/');
    if t.starts_with('/') {
        t.to_string()
    } else {
        format!("/{t}")
    }
}
