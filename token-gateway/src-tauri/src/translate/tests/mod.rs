//! `crate::translate` 的單元測試，按主題分檔：`anthropic`（Anthropic ⇄ OpenAI）、
//! `anthropic_reverse`（OpenAI → Anthropic，2026-10-10 補的反向）、
//! `gemini_reverse`（OpenAI → Gemini，2026-10-11 補的反向）、
//! `responses`（Responses ⇄ Chat 的請求／回應與工具）、`sanitize`（消息配對修復）、
//! `streaming`（Responses 流式事件）。

use super::*;

mod anthropic;
mod anthropic_reverse;
mod gemini_reverse;
mod responses;
mod sanitize;
mod streaming;
