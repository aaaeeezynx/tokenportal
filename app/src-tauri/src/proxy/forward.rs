//! 上游目標組裝、模型來源解析、請求體轉譯與剝離。
//!
//! 從「客戶端要什麼格式」到「上游收到什麼位元組」之間的所有轉換
//! 都在這裡。`TransSpec` 只帶兩個欄位（翻譯類型、是否翻譯）是刻意的：
//! 它曾帶著 `in_fmt`/`target_fmt` 卻沒人讀，clippy 抓到後才瘦身。

use super::*;

/// 跨來源路由命中行：改走來源的連接資訊。
pub(super) struct ReroutedProvider {
    pub(super) id: i64,
    pub(super) base_url: String,
    pub(super) api_key: String,
    pub(super) scheme: String,
    pub(super) format: String,
    /// 逐模型協議（provider_models.protocol）；空 = 未偵測
    pub(super) protocol: String,
    /// 逐協議路徑前綴的原始 JSON（空 = 用預設）
    pub(super) prefixes: String,
}
/// 組裝上游 URL 與轉發標頭，回傳 `(url, headers)`。
///
/// - 翻譯路徑固定打 `/chat/completions`；直通保留原始路徑（`join_upstream`
///   負責去重 `/v1` 之類的重複前綴）。查詢字串會剝掉 `key=` 再附加。
/// - 標頭：先複製客戶端標頭（略過 hop-by-hop），再依渠道 `auth_scheme`
///   注入鑑權 —— `goog-key` 用 `x-goog-api-key`、`anthropic` 用 `x-api-key`
///   + 版本頭、其餘用 `Bearer`。
pub(super) fn build_upstream_target(
    parts: &axum::http::request::Parts,
    authed: &keys::AuthedKey,
    translated: bool,
    app: &str,
    target_fmt: TargetFmt,
    // 翻譯器指定的上游路徑（含查詢字串）。`None` = 用下面的預設。
    //
    // Gemini 專用：它的模型名在路徑裡（`/models/{m}:generateContent`），
    // 而路徑長相取決於翻譯結果（串流與否），所以由翻譯器決定。
    path_override: Option<&str>,
) -> (String, Vec<(String, String)>) {
    let base = authed.provider_base_url.trim_end_matches('/');
    // 逐協議路徑前綴（見 `models/prefixes.rs`）。
    //
    // 有些上游把協議做在**路徑前綴**裡（實測 OpenCode Go：同一個 base 底下
    // `/openai/v1/…`、`/anthropic/v1/…`、`/google/v1beta/…`）。前綴接在 base 之後、
    // 協議字尾之前；空字串 = 沒有前綴（既有行為，向後相容）。
    let prefixes = crate::models::ProtocolPrefixes::parse(&authed.provider_protocol_prefixes);
    let prefix = prefixes.for_protocol(target_fmt.as_str());
    let mut url = if let Some(p) = path_override {
        format!("{base}{prefix}{p}")
    } else if translated {
        // 翻譯路徑的端點由**目標協議**決定，不能寫死 /chat/completions ——
        // Anthropic 上游（例如 opencode-go 的 /messages）是另一個路徑。
        if matches!(target_fmt, TargetFmt::Anthropic) {
            format!("{base}{prefix}/messages")
        } else {
            format!("{base}{prefix}/chat/completions")
        }
    } else if prefix.is_empty() {
        join_upstream(&authed.provider_base_url, parts.uri.path())
    } else {
        // 直通 ＋ 有前綴：{base}{prefix}{客戶端路徑去掉版本段}。
        //
        // **必須去掉版本段**：前綴本身就帶著版本（`/openai/v1`、`/google/v1beta`），
        // 而客戶端路徑也帶（`/v1/chat/completions`）—— 不去掉就會變成
        // `/openai/v1/v1/chat/completions`。
        format!("{base}{prefix}{}", strip_leading_version(parts.uri.path()))
    };
    // 覆寫路徑自己就帶查詢字串（`?alt=sse`），不能再接客戶端的 query
    if path_override.is_none() {
        if let Some(q) = strip_key_param(parts.uri.query()) {
            url.push('?');
            url.push_str(&q);
        }
    }

    let mut fwd: Vec<(String, String)> = Vec::new();
    for (k, v) in parts.headers.iter() {
        let name = k.as_str();
        if is_hop_header(name) {
            continue;
        }
        if let Ok(val) = v.to_str() {
            fwd.push((name.to_string(), val.to_string()));
        }
    }
    let pkey = authed.provider_api_key.as_str();
    // Anthropic 目標**一律補上** `x-api-key` + 版本頭，即使來源的 auth_scheme
    // 是 bearer。
    //
    // 為什麼：`/messages` 端點普遍只認 `x-api-key`。實測 opencode-go 用
    // `Authorization: Bearer` 打 `/messages` 會回 401 `Missing API key`，
    // 而同一個來源的 `/chat/completions` 認 Bearer。逐模型協議讓「一個來源
    // 同時服務兩種協議」成為常態之後，鑑權就必須跟著**目標協議**走，而不是
    // 只跟著來源走 —— 否則那個來源的 Anthropic 模型永遠 401。
    //
    // 用「補上」而不是「取代」：來源若本來就宣告 anthropic，下面的 match 會
    // 再加一次同樣的標頭（`reqwest` 以最後一個為準，值相同無害）；而某些
    // 代理只認 Bearer，兩者都送才能同時涵蓋。
    if matches!(target_fmt, TargetFmt::Anthropic) && !pkey.is_empty() {
        fwd.push(("x-api-key".to_string(), pkey.to_string()));
        fwd.push(("anthropic-version".to_string(), "2023-06-01".to_string()));
    }
    // Gemini 目標同理：`generateContent` 只認 `x-goog-api-key`。
    if matches!(target_fmt, TargetFmt::Gemini) && !pkey.is_empty() {
        fwd.push(("x-goog-api-key".to_string(), pkey.to_string()));
    }
    match authed.provider_auth_scheme.as_str() {
        "goog-key" => {
            // Google 原生：x-goog-api-key，不用 Bearer
            if !pkey.is_empty() {
                fwd.push(("x-goog-api-key".to_string(), pkey.to_string()));
            }
        }
        "anthropic" => {
            // Anthropic 官方：x-api-key + 版本頭
            if !pkey.is_empty() {
                fwd.push(("x-api-key".to_string(), pkey.to_string()));
            }
            fwd.push(("anthropic-version".to_string(), "2023-06-01".to_string()));
        }
        _ => {
            if !pkey.is_empty() {
                fwd.push(("authorization".to_string(), format!("Bearer {pkey}")));
            }
        }
    }

    // OpenCode Zen/Go 要求 session 標頭才能有效路由（缺失回 400 MissingSessionID）。
    // 客戶端（hermes/opencode/claude 等）沒帶時，注入穩定的合成 id：同一本地 Key
    // 固定同一 id → 上游路由與前綴快取穩定；Codex 自帶 session-id 時原樣透傳。
    if authed.provider_base_url.contains("opencode.ai") {
        const SESSION_HEADS: [&str; 6] = [
            "x-opencode-session",
            "session-id",
            "x-session-id",
            "thread-id",
            "x-client-request-id",
            "x-grok-session-id",
        ];
        let has_session = fwd
            .iter()
            .any(|(k, _)| SESSION_HEADS.iter().any(|h| k.eq_ignore_ascii_case(h)));
        if !has_session {
            let who = if app.is_empty() { "unknown" } else { app };
            let sid = format!("tg-{who}-{}", authed.id);
            fwd.push(("x-opencode-session".to_string(), sid.clone()));
            fwd.push(("session-id".to_string(), sid));
        }
    }
    // 注：X-TG-App 是網關內務頭，如需隱藏可在此剝離；當前選擇透傳以便上游觀測。
    (url, fwd)
}
#[derive(Debug, Clone, Copy)]
pub(super) struct TransSpec {
    pub(super) kind: TransKind,
    /// 是否需要翻譯（`kind` 不是 None，且渠道協議不是原生同型）。
    pub(super) translated: bool,
}
/// 要送往上游的請求體。
pub(super) struct ForwardBody {
    pub(super) bytes: Vec<u8>,
    /// 送往上游的實際模型名（翻譯或直通改寫時）；未改寫則為空字串。
    pub(super) model: String,
    /// 覆寫上游路徑（含查詢字串）。`None` = 用 [`build_upstream_target`] 的預設。
    ///
    /// 存在的理由：**Gemini 把模型名放在路徑裡**
    ///（`/models/{model}:generateContent` 或 `:streamGenerateContent?alt=sse`），
    /// 而路徑長相取決於翻譯結果（串流與否）。把它交給翻譯器決定，
    /// 而不是讓 `build_upstream_target` 去猜。
    pub(super) path_suffix: Option<String>,
}
/// `translate_forward_body` 的結果。
pub(super) enum BodyPrep {
    Ready(ForwardBody),
    /// 需要翻譯，但請求體不是合法 JSON —— 呼叫端負責留痕與回應。
    Unparsable,
}
/// 把客戶端請求體轉成要送上游的位元組。
///
/// - 翻譯路徑（Anthropic／Responses → chat）：以白名單重建 body，
///   同時把 model 由顯示名換成 actual。
/// - 直通路徑：只把 body 內既有的 `model` 字串由顯示名改寫成 actual
///   （無映射則原樣不動；Gemini 原生等無 model 的 body 完全不碰）。
/// - 最後若為 OpenAI chat 的串流請求，補 `stream_options.include_usage` ——
///   Responses／Anthropic／Gemini 加這個欄位會被上游 400，故以
///   `want_usage_opt` 區分。
pub(super) fn translate_forward_body(
    conn: &rusqlite::Connection,
    provider_id: i64,
    raw: &[u8],
    body_json: &Option<serde_json::Value>,
    model_raw: &str,
    fmt: TransSpec,
    want_usage_opt: bool,
) -> BodyPrep {
    let mut fwd_value = body_json.clone();
    let mut translated_model = String::new();
    let mut rewritten = false;
    // 翻譯器若需要覆寫上游路徑（Gemini 把模型名放在路徑裡）就填這裡
    let mut translated_path: Option<String> = None;

    if fmt.translated {
        let Some(v) = fwd_value.as_ref().cloned() else {
            return BodyPrep::Unparsable;
        };
        let actual = crate::models::resolve_actual(conn, provider_id, model_raw);
        // 思考強度只轉發「這個模型收得下」的檔位（見 models::allowed_efforts）
        let allowed = crate::models::allowed_efforts(conn, provider_id, &actual);
        fwd_value = Some(match fmt.kind {
            TransKind::ResponsesToChat => {
                crate::translate::responses_to_openai(&v, &actual, &allowed)
            }
            // Responses → Anthropic 是**兩跳**：先重用既有的 Responses→Chat，
            // 再接 Chat→Anthropic。刻意不另寫一份 Responses⇄Anthropic 的直接翻譯
            // ——那會是第二套要維護的 Responses 語意。
            TransKind::ResponsesToAnthropic => {
                let chat = crate::translate::responses_to_openai(&v, &actual, &allowed);
                crate::translate::chat_to_anthropic(&chat, &actual, &allowed)
            }
            TransKind::ChatToAnthropic => {
                crate::translate::chat_to_anthropic(&v, &actual, &allowed)
            }
            // Gemini 同理：Responses 走兩跳，Chat 直接跳
            TransKind::ResponsesToGemini => {
                let chat = crate::translate::responses_to_openai(&v, &actual, &allowed);
                let (body, path) = crate::translate::chat_to_gemini(&chat, &actual, &allowed);
                translated_path = Some(path);
                body
            }
            TransKind::ChatToGemini => {
                let (body, path) = crate::translate::chat_to_gemini(&v, &actual, &allowed);
                translated_path = Some(path);
                body
            }
            // Anthropic（Claude Code）→ Gemini 上游：同樣兩跳。
            // 先轉成 Chat 再轉 Gemini —— 刻意不寫第三份「Anthropic ⇄ Gemini」
            // 的直接翻譯，那會變成要維護第三套語意。
            TransKind::AnthropicToGemini => {
                let chat = crate::translate::anthropic_to_openai(&v, &actual);
                let (body, path) = crate::translate::chat_to_gemini(&chat, &actual, &allowed);
                translated_path = Some(path);
                body
            }
            _ => crate::translate::anthropic_to_openai(&v, &actual),
        });
        translated_model = actual;
    } else if !model_raw.is_empty() {
        // 直通改寫：僅當請求體本來就帶 model 字串才動
        if let Some(obj) = fwd_value.as_mut().and_then(|v| v.as_object_mut()) {
            if obj.get("model").and_then(|m| m.as_str()).is_some() {
                let actual = crate::models::resolve_actual(conn, provider_id, model_raw);
                if !actual.is_empty() && actual != model_raw {
                    obj.insert(
                        "model".to_string(),
                        serde_json::Value::String(actual.clone()),
                    );
                    translated_model = actual;
                    rewritten = true;
                }
            }
        }
    }

    let mut bytes = if fmt.translated || rewritten {
        serde_json::to_vec(fwd_value.as_ref().expect("forward body"))
            .unwrap_or_else(|_| raw.to_vec())
    } else {
        raw.to_vec()
    };
    // 串流強制索取用量（僅 OpenAI chat 需要；見上方說明）
    if want_usage_opt {
        if let Some(v) = fwd_value.as_ref() {
            if v.get("stream").and_then(|s| s.as_bool()).unwrap_or(false)
                && v.get("stream_options").is_none()
            {
                let mut owned = v.clone();
                if let Some(obj) = owned.as_object_mut() {
                    obj.insert(
                        "stream_options".to_string(),
                        serde_json::json!({"include_usage": true}),
                    );
                    bytes = serde_json::to_vec(&owned).unwrap_or(bytes);
                }
            }
        }
    }
    BodyPrep::Ready(ForwardBody {
        bytes,
        model: translated_model,
        path_suffix: translated_path,
    })
}
/// 去掉路徑開頭的版本段（`/v1`、`/v1beta`、`/v2` …）。
///
/// 只在「有設定逐協議前綴」的直通路徑用到：前綴本身已經帶版本，客戶端路徑也帶，
/// 不去掉就會拼出 `/openai/v1/v1/chat/completions`。
pub(super) fn strip_leading_version(path: &str) -> String {
    let mut segs = path.split('/').filter(|s| !s.is_empty());
    let first = segs.next().unwrap_or("");
    let is_version = first.len() >= 2
        && first.len() <= 8
        && first.starts_with(['v', 'V'])
        && first[1..].chars().all(|c| c.is_ascii_alphanumeric());
    let rest: Vec<&str> = if is_version {
        segs.collect()
    } else {
        std::iter::once(first).chain(segs).collect()
    };
    if rest.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", rest.join("/"))
    }
}

/// 依序套用所有「上游已知拒收」的剝離：
/// ① 該渠道學到的拒收欄位（SQLite 持久化記憶，見 §3 B2）
/// ② OpenAI 專屬推理簽章 `encrypted_content`（§3 B1）—— **有條件**，見下
/// ③ 上游不支援的 tool types（Codex `custom` → DeepSeek 400）
/// ④ 直通 chat 的歷史配對修復（舊 session 孤兒 tool_calls；有效歷史不動）
///
/// 每個步驟都是「有改才換、沒改不動」，所以全部 no-op 時 body 原樣送出。
pub(super) fn strip_for_upstream(
    conn: &rusqlite::Connection,
    provider_id: i64,
    mut body: Vec<u8>,
    in_fmt: InFmt,
    translated: bool,
) -> Vec<u8> {
    let learned = trace::load_stripped(conn, provider_id);
    let learned_enc = learned.iter().any(|f| f == ENCRYPTED_CONTENT_FIELD);
    for f in learned.iter().filter(|f| *f != ENCRYPTED_CONTENT_FIELD) {
        if let Some(nb) = strip_json_field(&body, f) {
            body = nb;
        }
    }
    // ② 推理簽章 `encrypted_content`：**不能無條件剝**。
    //
    // 2026-10-10 的真實故障：客戶端是 `store: false`（Codex 的
    // `disable_response_storage = true`）時，上游**不保存回應**，推理的重播
    // 完全依賴 `encrypted_content`。把它剝掉，上游就只剩一個查不到的 ID：
    // `Referenced reasoning item 'rs_…' was not found or has expired.`
    //
    // 那個剝離原本是為了修**另一個**真實問題：ChatGPT 授權模式下由 OpenAI
    // 發出的簽章，送到第三方上游會被回 `was not issued to this caller`。
    // 兩者的差別是「簽章是不是這個上游發的」，靜態分不出來 —— 所以：
    // - 無狀態請求（`store: false`）→ 保留（那是重播的唯一依據）
    // - 其他 → 照舊剝（涵蓋原本那個簽章問題）
    // - 已學到「這個來源就是會拒收簽章」→ 連無狀態也剝（見 proxy/retry.rs 的
    //   反應式學習，寫進 `provider_stripped_fields`）
    if learned_enc || !is_stateless(&body) {
        if let Some(stripped) = strip_encrypted_content(&body) {
            body = stripped;
        }
    }
    if !translated {
        if let Some(stripped) = strip_unsupported_tools(&body) {
            body = stripped;
        }
        if let Some(fixed) = sanitize_passthrough_chat_body(&body, in_fmt, translated) {
            body = fixed;
        }
    }
    body
}
