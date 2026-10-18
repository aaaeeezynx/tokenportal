//! M7：Anthropic Messages ⇄ OpenAI Chat Completions 翻譯。
//!
//! 原本只做 **Anthropic 進 → OpenAI 出**（Claude Code 經網關打 OpenAI 兼容上游，
//! 即用戶現有的 cc-switch 流程），反向回明確 400。2026-10-10 補上**反向**：
//! OpenAI 格式的客戶端（Codex、各種 Chat 工具）打 Anthropic 上游
//! （例如 opencode-go 的 `claude-haiku-5-5`）—— 見 `anthropic_req`／`anthropic_resp`。
//!
//! 模型名映射走可用模型表（display_name → actual_model），見 `models::resolve_actual`。
//!
//! 本模組按方向／主題拆成子模組：`openai_chat`（Anthropic 進 → OpenAI 出的請求側）、
//! `sanitize`（Chat 消息配對修復）、`anthropic`（OpenAI 回 → Anthropic，含流式）、
//! `anthropic_req`／`anthropic_resp`（OpenAI 進 → Anthropic 上游，含流式）、
//! `responses`＋`responses_tools`＋`responses_stream`＋`responses_finish`
//!（Responses ⇄ Chat Completions）。子模組的項目在此以 `pub(crate) use` 重新導出，
//! 因此 `crate::translate::X` 對每個原有名字都不變。

use serde_json::Value;

/// 流式/非流式共用的用量累積（代理記帳用）。
#[derive(Debug, Default, Clone)]
pub(crate) struct SseUsage {
    pub in_tok: i64,
    pub out_tok: i64,
    pub cache_read: i64,
    pub cache_write: i64,
    pub model: Option<String>,
}

impl SseUsage {
    pub fn total(&self) -> i64 {
        self.in_tok + self.out_tok + self.cache_read + self.cache_write
    }
}

pub(crate) fn num(v: &serde_json::Value, key: &str) -> Option<i64> {
    v.get(key)?.as_i64()
}

/// OpenAI usage 對象 → 累積（chat / responses 通用）。
pub(crate) fn read_openai_usage(u: &serde_json::Value, acc: &mut SseUsage) {
    if let Some(n) = num(u, "prompt_tokens").or(num(u, "input_tokens")) {
        acc.in_tok = n;
    }
    if let Some(n) = num(u, "completion_tokens").or(num(u, "output_tokens")) {
        acc.out_tok = n;
    }
    if let Some(n) = num(u, "cache_read_input_tokens") {
        acc.cache_read = n;
    }
    if let Some(n) = num(u, "cache_creation_input_tokens") {
        acc.cache_write = n;
    }
    if let Some(det) = u.get("prompt_tokens_details") {
        if let Some(n) = num(det, "cached_tokens") {
            acc.cache_read = n;
        }
    }
}

fn s(v: &Value) -> String {
    v.as_str().unwrap_or("").to_string()
}

/// 缺失的歷史 tool 輸出佔位（舊 session 續跑 / 中斷輪次常見）：讓上游配對校驗通過，
/// 模型按現有上下文繼續，不會把佔位當真實執行結果。
const MISSING_TOOL_OUTPUT: &str =
    "[Previous tool result unavailable — restored from session history; continue with available context.]";

/// 非 assistant 空消息佔位：部分嚴格上游（如 OpenRouter 後端 ModelRun）要求
/// 非 assistant 角色的 content 非空，空字串/缺失會 `message content is required
/// for non-assistant roles` 400（2026-09-24 codex 實例）。assistant 豁免
///（帶 tool_calls 的 content:null 與純空文本均為 OpenAI 合法形狀）。
const EMPTY_MESSAGE_PLACEHOLDER: &str = "[empty message — placeholder filled by gateway]";

/// 思考強度 → 上游的「延伸思考」預算（**兩個上游共用同一份映射**）。
///
/// - Anthropic：`thinking.budget_tokens`
/// - Gemini：`generationConfig.thinkingConfig.thinkingBudget`
///
/// 刻意**同構且單調**（檔位越高、預算越大）：使用者在兩個上游之間切換時，
/// 「high」的相對強度才是一致的。`minimal` 取 1024（Anthropic 的下限）。
/// 回傳 `None` = 不啟用延伸思考（`unset` 或上游不認的檔位）。
///
/// 放在共用層而不是各自的翻譯器裡，是因為兩邊**必須**一致 ——
/// 分成兩份遲早會漂移（而且會造成同名匯出衝突）。
/// Gemini 的 `thoughtSignature` 在本網關裡的攜帶方式。
///
/// 為什麼要繞這一圈：Gemini 要求把模型回傳的 `thoughtSignature` 在**下一輪
/// 原樣送回**（放在 `parts[].functionCall` 的**同層**）。缺了會直接 400：
///
/// ```text
/// [400] Function call is missing a thought_signature in functionCall parts.
/// ```
///
/// 而串流端點更惡劣 —— 同一個錯誤回的是**空 body**，所以查了半天只看到
/// 「上游返回錯誤（空回包）」。
///
/// Chat 與 Responses 都沒有能放這個欄位的型別，於是把它**夾在 tool-call id**
/// 裡往返（`base64` 的字母表不含 `~`，不會撞）：
///
/// ```text
/// id = call_4312684~ElkKVwFpFH0T7D2suEmwRF+Snrl2tRUlxiDTs6WXk+Oe9klVrlcohVRgKZYa==
/// ```
///
/// 客戶端只是把 id 原樣回傳，不必知道裡面有什麼。若 id 被客戶端改寫、
/// 或上一輪不是 Gemini 回的，就退回 [`SKIP_SIGNATURE`] —— 實測可行。
const SIGNATURE_SEP: char = '~';

/// Gemini 官方的「略過簽章驗證」哨兵。
///
/// 沒有真實簽章時必須送這個：**留空一定 400**，而隨便塞 base64 會得到
/// `Corrupted thought signature`。實測只有這個字串與真實簽章兩種會過。
pub(crate) const SKIP_SIGNATURE: &str = "skip_thought_signature_validator";

/// 從 tool-call id 取出夾帶的簽章。
pub(crate) fn signature_from_id(id: &str) -> Option<String> {
    let (_, sig) = id.rsplit_once(SIGNATURE_SEP)?;
    if sig.is_empty() {
        None
    } else {
        Some(sig.to_string())
    }
}

/// 把簽章夾進 tool-call id。
pub(crate) fn id_with_signature(base: &str, signature: &str) -> String {
    if signature.is_empty() {
        base.to_string()
    } else {
        format!("{base}{SIGNATURE_SEP}{signature}")
    }
}

pub(crate) fn thinking_budget(effort: &str) -> Option<i64> {
    match effort.trim().to_ascii_lowercase().as_str() {
        "minimal" => Some(1_024),
        "low" => Some(2_048),
        "medium" => Some(8_192),
        "high" => Some(16_384),
        "xhigh" => Some(32_768),
        "max" | "ultra" | "persistent" => Some(65_536),
        _ => None,
    }
}

mod anthropic;
mod anthropic_req;
mod anthropic_resp;
mod gemini_req;
mod gemini_schema;
mod gemini_resp;
mod openai_chat;
mod responses;
mod responses_finish;
mod responses_stream;
mod responses_tools;
mod sanitize;

pub(crate) use anthropic::*;
pub(crate) use anthropic_req::*;
pub(crate) use anthropic_resp::*;
pub(crate) use gemini_req::*;
pub(crate) use gemini_resp::*;
pub(crate) use openai_chat::*;
pub(crate) use responses::*;
pub(crate) use responses_finish::*;
pub(crate) use responses_stream::*;
pub(crate) use responses_tools::*;
pub(crate) use sanitize::*;

#[cfg(test)]
mod tests;
