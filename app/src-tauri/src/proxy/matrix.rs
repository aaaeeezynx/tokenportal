//! 格式矩陣：入站格式 × 渠道協議 → 翻譯類型。
//!
//! 從 `proxy.rs` 拆出（Phase 2 第十一步）。這裡是 §3 B5/B6 兩個 bug 的所在，
//! 也是本專案「用 enum + 窮舉 match，讓編譯器保證矩陣正確性」這個設計的實體：
//! 少列一格就編譯失敗，從根本上消除「正確性依賴分支順序」的脆弱性。
//!
//! 本模組**不依賴父模組**（純字串比對），故無 `use super::*`。


/// 入站→渠道翻譯類型（None = 原樣透傳）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TransKind {
    None,
    AnthropicToChat,
    ResponsesToChat,
    /// OpenAI Chat 請求 → Anthropic 上游（2026-10-10 補的反向）
    ChatToAnthropic,
    /// Responses（Codex）請求 → Anthropic 上游。請求側是兩跳組合：
    /// `responses_to_openai` 再接 `chat_to_anthropic`。
    ResponsesToAnthropic,
    /// OpenAI Chat 請求 → Gemini 上游（2026-10-11）。
    ChatToGemini,
    /// Responses（Codex）請求 → Gemini 上游。同樣是兩跳：
    /// `responses_to_openai` 再接 `chat_to_gemini`。
    ResponsesToGemini,
    /// Anthropic（Claude Code）請求 → Gemini 上游（2026-10-20）。
    ///
    /// 補上這格之後，「三種客戶端 × 四種上游」全部可達 —— 剩下的
    /// `Gemini 入站` 是刻意鎖死的（沒有 Gemini 原生客戶端在用）。
    ///
    /// 請求側兩跳：`anthropic_to_openai` 再接 `chat_to_gemini`。
    /// 回應側兩跳：`gemini_chunk_to_chat_chunks` 再接 `openai_chunk_to_anthropic`
    /// （串流與非串流各一條，都是既有零件）。
    AnthropicToGemini,
}

impl TransKind {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            TransKind::None => "none",
            TransKind::AnthropicToChat => "anthropic_to_chat",
            TransKind::ResponsesToChat => "responses_to_chat",
            TransKind::ChatToAnthropic => "chat_to_anthropic",
            TransKind::ResponsesToAnthropic => "responses_to_anthropic",
            TransKind::ChatToGemini => "chat_to_gemini",
            TransKind::ResponsesToGemini => "responses_to_gemini",
            TransKind::AnthropicToGemini => "anthropic_to_gemini",
        }
    }
}

/// 入站格式（由請求路徑判定）。
///
/// 改用 enum（原為 `&str`）是為了讓格式矩陣能被**編譯器**窮舉檢查：
/// `match (InFmt, TargetFmt)` 若少列一格就編譯失敗，從根本上消除
/// 「正確性依賴分支順序」的脆弱性（見 docs/REFACTORING-PLAN.md §3 B5）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum InFmt {
    Anthropic,
    Responses,
    OpenAi,
    Gemini,
    /// 未知／中性路徑（如 `/v1/models`）：一律透傳，不誤傷。
    Unknown,
}

impl InFmt {
    pub(super) fn from_path(path: &str) -> Self {
        if path.contains(":generateContent")
            || path.contains(":streamGenerateContent")
            || path.contains(":embedContent")
            || path.contains("v1beta/models")
        {
            InFmt::Gemini
        } else if path.ends_with("/messages") {
            InFmt::Anthropic
        } else if path.ends_with("/responses") {
            InFmt::Responses
        } else if path.contains("/chat/completions")
            || path.contains("/embeddings")
            || path.ends_with("/models")
        {
            InFmt::OpenAi
        } else {
            InFmt::Unknown
        }
    }

    pub(super) fn as_str(self) -> &'static str {
        match self {
            InFmt::Anthropic => "anthropic",
            InFmt::Responses => "responses",
            InFmt::OpenAi => "openai",
            InFmt::Gemini => "gemini",
            InFmt::Unknown => "unknown",
        }
    }

    /// 給使用者看的格式名（錯誤訊息用）。
    pub(super) fn label(self) -> &'static str {
        match self {
            InFmt::Anthropic => "Anthropic",
            InFmt::Responses => "Responses",
            InFmt::OpenAi => "OpenAI Chat",
            InFmt::Gemini => "Gemini",
            InFmt::Unknown => "入站",
        }
    }
}

/// 渠道協議（`providers.api_format`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TargetFmt {
    OpenAiChat,
    OpenAiResponses,
    /// 混合渠道：**兩種端點都原樣直通**（見 `db.rs` 的 zen 種子）。
    /// chat 入站 → `/chat/completions`；Responses 入站（Codex）→ `/responses`。
    ///
    /// 注意這是**渠道級**的「兩邊都通」：**逐模型**的端點支援仍由上游決定 ——
    /// 只有當該來源的模型在客戶端用的那個端點上有架時才不會 `ModelProtocolUnsupported`
    /// （2026-09-30 實測 oc-go：grok-4.7 只在 responses、mimo-v2.6 只在 chat）。
    Mixed,
    Anthropic,
    Gemini,
    /// `auto`（2026-10-10）：**協議逐模型決定**，由 `provider_models.protocol`
    /// 或入站格式推導（見 `failover::formats_for`）。
    ///
    /// 這是新來源的預設值。動機：協議是「模型在某個上游」的屬性，不是「來源」
    /// 的屬性 —— 訂閱制的 opencode-go 單一 base_url 底下 Claude 走 `/messages`、
    /// 其餘走 `/chat/completions`，用來源級欄位描述不了，使用者只能把同一個
    /// 訂閱拆成兩個來源（金鑰重複、統計被切碎）。
    ///
    /// **這一格不該被走到**：`formats_for` 會先把 `auto` 展開成具體協議再排候選。
    /// 留著是為了讓 match 保持窮舉（少列一格就編譯失敗）。
    Auto,
    /// `api_format` 不在已知值內（拼錯或未來新增）。**不猜測協議**。
    Unknown,
}

impl TargetFmt {
    pub(super) fn from_db(s: &str) -> Self {
        match s {
            "openai-chat" => TargetFmt::OpenAiChat,
            "openai-responses" => TargetFmt::OpenAiResponses,
            "mixed" => TargetFmt::Mixed,
            "anthropic" => TargetFmt::Anthropic,
            "gemini" => TargetFmt::Gemini,
            "auto" => TargetFmt::Auto,
            _ => TargetFmt::Unknown,
        }
    }

    pub(super) fn as_str(self) -> &'static str {
        match self {
            TargetFmt::OpenAiChat => "openai-chat",
            TargetFmt::OpenAiResponses => "openai-responses",
            TargetFmt::Mixed => "mixed",
            TargetFmt::Anthropic => "anthropic",
            TargetFmt::Gemini => "gemini",
            TargetFmt::Auto => "auto",
            TargetFmt::Unknown => "unknown",
        }
    }
}

// 400 訊息（使用者直接看到，故集中管理）
pub(super) const E_ANTHROPIC_UNSUPPORTED: &str =
    "該渠道不接受 Anthropic 請求：請換 Anthropic 官方或 OpenAI 兼容渠道";
// `E_REVERSE_UNSUPPORTED`（「反向轉換尚未實現」）已於 2026-10-10 移除：
// OpenAI → Anthropic 的翻譯器已實作，兩個矩陣格不再是 Err。
pub(super) const E_GEMINI_IN_ONLY: &str = "Gemini 原生請求只能走 Gemini 渠道";
pub(super) const E_GEMINI_OUT_ONLY: &str = "Gemini 渠道只接受 Gemini 原生請求";
/// B6：chat 請求打到 responses 端點，原實作靜默直通而必然失敗，改為明確報錯。
pub(super) const E_CHAT_TO_RESPONSES: &str =
    "此渠道為 Responses 協議，不接受 OpenAI Chat 請求：請改用 Chat 兼容渠道，或改用 Codex 等 Responses 客戶端";
/// `auto` 不該走到矩陣：`formats_for` 會先把它展開成具體協議。
/// 走到這裡代表內部不變式被破壞（例如新增了沒走 `formats_for` 的呼叫路徑）。
pub(super) const E_AUTO_UNRESOLVED: &str =
    "內部錯誤：auto 協議未被解析（請回報此訊息）";

/// 入站格式 × 渠道協議 → 翻譯類型（`Err` = 該組合不支援，字串即 400 訊息）。
///
/// **30 格全列舉，無萬用字元** —— 未來新增任何 `InFmt` / `TargetFmt` 變體，
/// 編譯器都會強制在此補上對應行為，不會靜默落到透傳。
/// 既有行為已逐格核對與原實作等價（僅 B6 一格由「靜默直通」改為「明確報錯」；
/// 2026-09-30 再把 `(Responses, Mixed)` 由 chat 翻譯改為原樣直通，兌現「兩邊都通」）。
pub(super) fn resolve_trans_kind(in_fmt: InFmt, target: TargetFmt) -> Result<TransKind, &'static str> {
    use InFmt::*;
    use TargetFmt as T;
    match (in_fmt, target) {
        // ── Anthropic 入站 ──
        (Anthropic, T::Anthropic) => Ok(TransKind::None),
        (Anthropic, T::OpenAiChat) | (Anthropic, T::Mixed) => Ok(TransKind::AnthropicToChat),
        // 2026-10-20：Gemini 上游也能接了。請求 anthropic_to_openai → chat_to_gemini，
        // 回應 gemini_chunk_to_chat_chunks → openai_chunk_to_anthropic（都是既有零件）。
        // 這格是「Claude Code 用 Gemini-only 模型」的唯一路徑（實測 opencode-go 的
        // gemini-4-argon-contributor 只吃原生 gemini，chat/responses/anthropic 全 400
        // ModelProtocolUnsupported）。
        (Anthropic, T::Gemini) => Ok(TransKind::AnthropicToGemini),
        (Anthropic, T::OpenAiResponses) | (Anthropic, T::Unknown) => {
            Err(E_ANTHROPIC_UNSUPPORTED)
        }
        (Anthropic, T::Auto) => Err(E_AUTO_UNRESOLVED),

        // ── Responses 入站（Codex）──
        (Responses, T::OpenAiChat) => Ok(TransKind::ResponsesToChat),
        // mixed = 上游兩種端點都有架，Responses 就別翻譯，原樣打過去（`forward.rs`
        // 的直通保留原始路徑）。翻譯成 chat 會讓 responses-only 的模型一律 400。
        (Responses, T::Mixed) => Ok(TransKind::None),
        (Responses, T::OpenAiResponses) => Ok(TransKind::None),
        // 2026-10-10：反向翻譯器已實作（見 translate/anthropic_req.rs）。
        // 請求側是兩跳：responses_to_openai → chat_to_anthropic。
        (Responses, T::Anthropic) => Ok(TransKind::ResponsesToAnthropic),
        // 2026-10-11：Gemini 的反向翻譯器已實作（見 translate/gemini_req.rs）
        (Responses, T::Gemini) => Ok(TransKind::ResponsesToGemini),
        (Responses, T::Auto) => Err(E_AUTO_UNRESOLVED),
        (Responses, T::Unknown) => Ok(TransKind::None),

        // ── OpenAI Chat 入站 ──
        (OpenAi, T::OpenAiChat) | (OpenAi, T::Mixed) => Ok(TransKind::None),
        (OpenAi, T::OpenAiResponses) => Err(E_CHAT_TO_RESPONSES),
        (OpenAi, T::Anthropic) => Ok(TransKind::ChatToAnthropic),
        (OpenAi, T::Gemini) => Ok(TransKind::ChatToGemini),
        (OpenAi, T::Auto) => Err(E_AUTO_UNRESOLVED),
        (OpenAi, T::Unknown) => Ok(TransKind::None),

        // ── Gemini 原生入站 ──
        (Gemini, T::Gemini) => Ok(TransKind::None),
        (Gemini, T::OpenAiChat)
        | (Gemini, T::OpenAiResponses)
        | (Gemini, T::Mixed)
        | (Gemini, T::Anthropic)
        | (Gemini, T::Auto)
        | (Gemini, T::Unknown) => Err(E_GEMINI_IN_ONLY),

        // ── 未知入站格式（中性路徑）──
        (Unknown, T::Gemini) => Err(E_GEMINI_OUT_ONLY),
        (Unknown, T::OpenAiChat)
        | (Unknown, T::OpenAiResponses)
        | (Unknown, T::Mixed)
        | (Unknown, T::Anthropic)
        | (Unknown, T::Unknown) => Ok(TransKind::None),
        // 中性路徑（如 /v1/models）：auto 就照原樣透傳，不猜協議
        (Unknown, T::Auto) => Ok(TransKind::None),
    }
}
