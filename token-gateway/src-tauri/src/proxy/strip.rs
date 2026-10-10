//! 請求體手術：未知欄位解析與各種「上游拒收就剝掉」的剝離。
//!
//! 這個模組是 §5.2「事後剝離重試」策略的實作面。每一支都回傳
//! `Option<Vec<u8>>`：`None` 代表「沒東西可剝」，呼叫端據此決定
//! 要不要重試 —— 這個約定是重試邏輯不會無窮迴圈的關鍵。

use super::*;

/// 從拒收錯誤訊息提取欄位名，支援兩種口徑：
/// - New-API 系：`unknown field "xxx"`（單個；上游回包是 JSON，訊息裡的引號是
///   轉義的 `\"`，因此按 token 抓取而非找引號）
/// - OpenRouter：`Unsupported parameter(s): 'xxx'`（可能多個：`'a', 'b'`；
///   Codex 的 prompt_cache_key 經此口徑被拒，2026-09-24 實例）
pub(super) fn parse_unknown_fields(msg: &str) -> Vec<String> {
    let is_name = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '-';
    if let Some(i) = msg.find("unknown field") {
        let rest = &msg[i + "unknown field".len()..];
        let start = match rest.find(is_name) {
            Some(s) => s,
            None => return vec![],
        };
        let end = rest[start..]
            .find(|c: char| !is_name(c))
            .map(|e| start + e)
            .unwrap_or(rest.len());
        let name = &rest[start..end];
        if name.is_empty() || name.len() > 64 {
            return vec![];
        }
        return vec![name.to_string()];
    }
    if let Some(i) = msg.find("Unsupported parameter") {
        let mut out: Vec<String> = vec![];
        let mut rest = &msg[i..];
        while let Some(q) = rest.find(['\'', '`', '"']) {
            let quote = rest.as_bytes()[q] as char;
            let after = &rest[q + quote.len_utf8()..];
            let Some(e) = after.find(quote) else {
                break;
            };
            let name = &after[..e];
            if !name.is_empty()
                && name.len() <= 64
                && name.chars().all(is_name)
                && !out.iter().any(|n| n == name)
            {
                out.push(name.to_string());
            }
            rest = &after[e + quote.len_utf8()..];
        }
        return out;
    }
    vec![]
}
/// 解析請求/轉發體 JSON，容忍 UTF-8 BOM（見 body_json 處注記）。
pub(super) fn parse_body_json(body: &[u8]) -> Option<serde_json::Value> {
    serde_json::from_slice(body.strip_prefix(b"\xef\xbb\xbf").unwrap_or(body)).ok()
}
/// 移除 JSON 頂層欄位；若不存在再試 `text.<field>`（Codex 把 verbosity
/// 放在 text 裡，置空後整個 text 一併移除）。
/// 嚴格上游拒收的新欄位時用。任一處命中即返回新 body。
pub(super) fn strip_json_field(body: &[u8], field: &str) -> Option<Vec<u8>> {
    let mut v: serde_json::Value = parse_body_json(body)?;
    let obj = v.as_object_mut()?;
    if obj.remove(field).is_some() {
        return serde_json::to_vec(&v).ok();
    }
    if let Some(text) = obj.get_mut("text") {
        if let Some(tobj) = text.as_object_mut() {
            if tobj.remove(field).is_some() {
                if tobj.is_empty() {
                    obj.remove("text");
                }
                return serde_json::to_vec(&v).ok();
            }
        }
    }
    None
}
/// 從請求體中過濾 `tools` 陣列，只保留上游支援的 tool type（`function`）。
/// Codex 會發 `type: "custom"` 等工具，DeepSeek 等上游不認會 400。
pub(super) fn strip_unsupported_tools(body: &[u8]) -> Option<Vec<u8>> {
    let mut v: serde_json::Value = parse_body_json(body)?;
    // 先過濾 tools，限制可變借用作用域
    let (new_len, original_len) = {
        let obj = v.as_object_mut()?;
        let tools = obj.get_mut("tools")?.as_array_mut()?;
        let orig = tools.len();
        tools.retain(|t| {
            t.get("type")
                .and_then(|tt| tt.as_str())
                .is_some_and(|tt| tt == "function")
        });
        (tools.len(), orig)
    };
    if new_len == 0 {
        let obj = v.as_object_mut().unwrap();
        obj.remove("tools");
        obj.remove("tool_choice");
        return serde_json::to_vec(&v).ok();
    }
    if new_len == original_len {
        return None;
    }
    // 檢查 tool_choice 引用的 tool 是否還在（只讀借用，不與 tools 衝突）
    let tc_name: Option<String> = v
        .get("tool_choice")
        .and_then(|tc| tc.get("name"))
        .and_then(|n| n.as_str())
        .map(|s| s.to_string());
    if let Some(name) = tc_name {
        let still_has = v["tools"]
            .as_array()
            .map(|tools| {
                tools.iter().any(|t| {
                    t.get("function")
                        .and_then(|f| f.get("name"))
                        .and_then(|n| n.as_str())
                        == Some(&name)
                })
            })
            .unwrap_or(false);
        if !still_has {
            v.as_object_mut().unwrap().remove("tool_choice");
        }
    }
    serde_json::to_vec(&v).ok()
}
/// 學到「這個來源拒收推理簽章」時，記在 `provider_stripped_fields` 用的欄位名。
///
/// 它不是一般的頂層欄位（`encrypted_content` 嵌在 reasoning item 裡），
/// 所以 `strip_for_upstream` 對它**特別處理**：改用 `strip_encrypted_content`
/// 遞迴移除，而不是 `strip_json_field` 只拔頂層。
pub(super) const ENCRYPTED_CONTENT_FIELD: &str = "encrypted_content";

/// 這個請求是不是「無狀態」的（`store: false`）。
///
/// **為什麼要判斷它**：`store: false` 時上游不保存回應，推理的重播完全依賴
/// `encrypted_content`；此時把它剝掉，上游就只剩一個查不到的 ID，回
/// `Referenced reasoning item 'rs_…' was not found or has expired.`
/// （2026-10-10 的實例：Codex 的 `disable_response_storage = true`）。
///
/// 用位元組掃描而不是完整 JSON parse：這個判斷在每個請求的熱路徑上，
/// 而且只需要找一個頂層布林值。容忍 `"store":false` 與 `"store": false`。
pub(super) fn is_stateless(body: &[u8]) -> bool {
    const KEY: &[u8] = b"\"store\"";
    let mut i = 0usize;
    while let Some(pos) = body[i..]
        .windows(KEY.len())
        .position(|w| w == KEY)
    {
        let mut j = i + pos + KEY.len();
        // 跳過空白 → 期待 ':' → 再跳空白 → 期待 false
        while j < body.len() && body[j].is_ascii_whitespace() {
            j += 1;
        }
        if j >= body.len() || body[j] != b':' {
            i = j;
            continue;
        }
        j += 1;
        while j < body.len() && body[j].is_ascii_whitespace() {
            j += 1;
        }
        if body[j..].starts_with(b"false") {
            return true;
        }
        i = j;
    }
    false
}

/// 上游錯誤訊息是不是「這個推理簽章不是我發的」。
///
/// OpenAI 系的講法是 `reasoning \`encrypted_content\` was not issued to this
/// caller`；只認這個片語，避免把別種 400 誤判成要剝簽章。
pub(super) fn is_encrypted_content_rejected(msg: &str) -> bool {
    msg.contains("was not issued to this caller")
}

/// 遞迴移除所有 `encrypted_content` 欄位（含 reasoning item 內）。
///
/// 目的：Codex 在 ChatGPT 授權模式下產生的推理簽章只對 OpenAI 本身有效，
/// 送到任何第三方 responses 端點都會被 400 拒收：
/// `reasoning \`encrypted_content\` was not issued to this caller`。
///
/// **呼叫端要注意時機**：`store: false` 的請求不能剝（那會讓上游無法重播
/// 推理），所以 `strip_for_upstream` 會先問 [`is_stateless`]。
/// 兩條翻譯路徑都已是白名單重建 body，天然不含此欄位。
/// 回傳 `None` 表示未命中 —— 原文一字不動。
pub(super) fn strip_encrypted_content(body: &[u8]) -> Option<Vec<u8>> {
    // 位元組快掃短路：絕大多數請求不含此欄位，避免每次多一次完整 JSON parse。
    const NEEDLE: &[u8] = b"encrypted_content";
    if !body.windows(NEEDLE.len()).any(|w| w == NEEDLE) {
        return None;
    }
    pub(super) fn walk(v: &mut serde_json::Value) -> bool {
        match v {
            serde_json::Value::Object(o) => {
                let mut hit = o.remove("encrypted_content").is_some();
                for (_, child) in o.iter_mut() {
                    // 不用 `|=` 短路：即使已命中也要走完全樹，移除所有副本
                    if walk(child) {
                        hit = true;
                    }
                }
                hit
            }
            serde_json::Value::Array(a) => {
                let mut hit = false;
                for c in a.iter_mut() {
                    if walk(c) {
                        hit = true;
                    }
                }
                hit
            }
            _ => false,
        }
    }
    let mut v: serde_json::Value = parse_body_json(body)?;
    if walk(&mut v) {
        serde_json::to_vec(&v).ok()
    } else {
        None
    }
}
/// 直通 chat 請求的歷史配對修復（舊 session 續跑常見孤兒 tool_calls）。
/// 僅處理 OpenAI chat 格式且未經翻譯的請求體；有效歷史返回 None（原文一字不動）。
pub(super) fn sanitize_passthrough_chat_body(
    body: &[u8],
    in_fmt: InFmt,
    translated: bool,
) -> Option<Vec<u8>> {
    if translated || in_fmt != InFmt::OpenAi {
        return None;
    }
    let v: serde_json::Value = parse_body_json(body)?;
    let arr = v.get("messages")?.as_array()?.clone();
    let fixed = crate::translate::sanitize_chat_messages(arr.clone());
    if fixed == arr {
        return None;
    }
    let mut obj = v.as_object()?.clone();
    obj.insert(
        "messages".to_string(),
        serde_json::Value::Array(fixed),
    );
    serde_json::to_vec(&serde_json::Value::Object(obj)).ok()
}
/// 上游錯誤原文透出（截斷，便於排查；不含密鑰）。
pub(super) fn upstream_err_text(bytes: &[u8]) -> String {
    let s = String::from_utf8_lossy(bytes);
    let t = s.trim();
    if t.is_empty() {
        return "上游返回錯誤（空回包）".to_string();
    }
    const MAX: usize = 300;
    if t.len() > MAX {
        format!("{}…", &t[..MAX])
    } else {
        t.to_string()
    }
}
