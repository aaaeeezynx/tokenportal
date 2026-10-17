//! 逐協議路徑前綴（`providers.protocol_prefixes`，2026-10-11）。
//!
//! 為什麼需要：有些上游把協議做在**路徑前綴**裡而不是端點字尾 —— 實測 OpenCode Go
//! 同一個 base 底下 `/openai/v1/…`、`/anthropic/v1/…`、`/google/v1beta/…`。
//! 用單一 `base_url` 加字尾描述不了，使用者只能為每種協議各開一個來源。

use super::*;

fn authed(prefixes: &str) -> keys::AuthedKey {
    keys::AuthedKey {
        id: 1,
        rate_limit_qpm: 0,
        allowed_models: vec![],
        allowed_apps: vec![],
        provider_id: 7,
        provider_base_url: "https://example.test/go".into(),
        provider_api_key: "k".into(),
        provider_auth_scheme: "bearer".into(),
        provider_api_format: "auto".into(),
        provider_protocol_prefixes: prefixes.into(),
        direct: false,
    }
}

fn parts(path: &str) -> axum::http::request::Parts {
    axum::http::Request::builder()
        .uri(path)
        .body(())
        .unwrap()
        .into_parts()
        .0
}

const OC: &str = r#"{"openai-chat":"/openai/v1","openai-responses":"/openai/v1",
                     "anthropic":"/anthropic/v1","gemini":"/google/v1beta"}"#;

/// ★ 核心：四種協議各自拼出正確的 URL —— **一個來源服務全部四種**。
#[test]
fn prefixes_build_the_four_protocol_urls() {
    let a = authed(OC);
    // 翻譯路徑（chat）→ {base}{prefix}/chat/completions
    let (u, _) = build_upstream_target(
        &parts("/v1/responses"),
        &a,
        true,
        "codex",
        TargetFmt::OpenAiChat,
        None,
    );
    assert_eq!(u, "https://example.test/go/openai/v1/chat/completions");
    // 翻譯路徑（anthropic）→ {base}{prefix}/messages
    let (u, _) = build_upstream_target(
        &parts("/v1/responses"),
        &a,
        true,
        "codex",
        TargetFmt::Anthropic,
        None,
    );
    assert_eq!(u, "https://example.test/go/anthropic/v1/messages");
    // Gemini：翻譯器給的路徑（含模型名）
    let (u, _) = build_upstream_target(
        &parts("/v1/responses"),
        &a,
        true,
        "codex",
        TargetFmt::Gemini,
        Some("/models/gemini-x:generateContent"),
    );
    assert_eq!(
        u,
        "https://example.test/go/google/v1beta/models/gemini-x:generateContent"
    );
    // 直通（chat 客戶端 → chat 上游）：版本段要**去掉**，否則會 /v1/v1/
    let (u, _) = build_upstream_target(
        &parts("/v1/chat/completions"),
        &a,
        false,
        "opencode",
        TargetFmt::OpenAiChat,
        None,
    );
    assert_eq!(u, "https://example.test/go/openai/v1/chat/completions", "{u}");
}

/// 沒有前綴時**行為完全不變**（向後相容）。
#[test]
fn empty_prefixes_keep_the_old_behaviour() {
    let a = authed("");
    let (u, _) = build_upstream_target(
        &parts("/v1/chat/completions"),
        &a,
        false,
        "opencode",
        TargetFmt::OpenAiChat,
        None,
    );
    assert_eq!(u, "https://example.test/go/v1/chat/completions");
    let (u, _) = build_upstream_target(
        &parts("/v1/responses"),
        &a,
        true,
        "codex",
        TargetFmt::OpenAiChat,
        None,
    );
    assert_eq!(u, "https://example.test/go/chat/completions");
}

/// 壞掉的 JSON 當成「沒有前綴」—— **絕不讓設定錯誤擋住請求**。
#[test]
fn malformed_prefixes_fall_back_to_no_prefix() {
    let a = authed("{not json");
    let (u, _) = build_upstream_target(
        &parts("/v1/responses"),
        &a,
        true,
        "codex",
        TargetFmt::OpenAiChat,
        None,
    );
    assert_eq!(u, "https://example.test/go/chat/completions");
}

/// 前綴的正規化：缺斜線補、多餘斜線去掉、簡寫協議名也認。
#[test]
fn prefixes_are_normalized_and_accept_short_names() {
    let p = crate::models::ProtocolPrefixes::parse(
        r#"{"chat":"/openai/v1/","responses":"openai/v1","gemini":"google/v1beta/"}"#,
    );
    assert_eq!(p.for_protocol("openai-chat"), "/openai/v1");
    assert_eq!(p.for_protocol("chat"), "/openai/v1");
    assert_eq!(p.for_protocol("openai-responses"), "/openai/v1");
    // 沒填的協議 → 空（沒有前綴）
    assert_eq!(p.for_protocol("anthropic"), "");
    // 未知協議 → 空
    assert_eq!(p.for_protocol("bogus"), "");
}

/// 版本段只去開頭那一段，不會誤刪模型名裡的 `v…`。
#[test]
fn only_the_leading_version_segment_is_stripped() {
    assert_eq!(
        strip_leading_version("/v1/chat/completions"),
        "/chat/completions"
    );
    assert_eq!(
        strip_leading_version("/v1beta/models/gemini-x:generateContent"),
        "/models/gemini-x:generateContent"
    );
    assert_eq!(strip_leading_version("/chat/completions"), "/chat/completions");
    assert_eq!(
        strip_leading_version("/models/v1-thing:generateContent"),
        "/models/v1-thing:generateContent",
        "非開頭的 v… 不該被動"
    );
}
