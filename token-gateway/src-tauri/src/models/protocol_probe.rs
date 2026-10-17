// ───────────────────────────────────────────── 協議探測（2026-10-10）───
//
// 為什麼需要：**同一個來源可以逐模型混用協議**。實測 opencode-go 的
// `claude-haiku-5-5` 只吃 Anthropic Messages（`/messages` + `x-api-key`），
// 同來源的 `deepseek-*` 吃 OpenAI chat。但 `providers.api_format` 是
// **來源級**設定，表達不了這件事 —— 於是目錄把一個 Codex 永遠到不了的模型
// 也提供出去，使用者選了只會拿到「此模型不支援本協議」。
//
// 探測結果寫進既有的 `provider_model_protocol`（宣告鍵用哨兵
// [`PROBED_PROTOCOL_KEY`]，不會干擾換手用的逐宣告記憶）。

use rusqlite::Connection;
use serde::Serialize;

use super::probe::{client, ProbeCfg};

/// 協議探測寫進 `provider_model_protocol` 時用的宣告鍵（哨兵）。
///
/// 換手用的鍵是**來源實際宣告的協議**（`openai-chat` 等），所以這個值
/// 永遠不會與它們相撞；`load_protocol` 是精確比對，也不會被它影響。
pub(crate) const PROBED_PROTOCOL_KEY: &str = "__probe__";

/// 四種協議的候選（依「最可能」排序；來源宣告的那種會被提到最前面）。
///
/// `gemini` 是 2026-10-11 補的：在那之前 Gemini 模型探測後只會回
/// 「三種都不通」，那是誤導 —— 它其實只是第四種。
const PROTOCOLS: [&str; 4] = ["chat", "responses", "anthropic", "gemini"];

/// 來源的 `api_format` → 探測用的協議名（把最可能的排到第一個，省請求）。
fn declared_probe_name(format: &str) -> Option<&'static str> {
    match format.trim() {
        "openai-chat" => Some("chat"),
        "openai-responses" => Some("responses"),
        "anthropic" => Some("anthropic"),
        "gemini" => Some("gemini"),
        // `mixed` / `auto` 沒有單一答案：保持原順序
        _ => None,
    }
}

/// 實際要試的順序：來源若明確宣告了某種協議，先試它。
pub(crate) fn probe_order(format: &str) -> Vec<&'static str> {
    let mut v: Vec<&'static str> = PROTOCOLS.to_vec();
    if let Some(want) = declared_probe_name(format) {
        if let Some(i) = v.iter().position(|p| *p == want) {
            v.swap(0, i);
        }
    }
    v
}

/// 一次協議嘗試的結果分類。
///
/// **`inconclusive` 與 `rejected` 必須分開**：`5xx`／逾時／限流是上游暫時
/// 故障，不能推論成「這個模型不支援這個協議」。2026-10-10 由真機測試抓到：
/// `deepseek-v4.1-flash` 的 chat／responses 剛好回 502，探測器就誤判它
/// 「只走 Anthropic」—— 而目錄排除是**破壞性**的，會把一個好好的模型下架。
fn classify(status: u16) -> &'static str {
    if (200..300).contains(&status) {
        "ok"
    } else if (400..500).contains(&status) && status != 429 {
        "rejected"
    } else {
        "inconclusive" // 5xx／429／0（連線失敗）
    }
}

/// 一次協議嘗試。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ProtocolAttempt {
    /// `chat` / `responses` / `anthropic`
    pub protocol: String,
    pub status: u16,
    /// `ok` / `rejected` / `inconclusive`
    pub outcome: String,
    /// 上游回應摘要（人話）
    pub detail: String,
}

/// 協議探測報告。
#[derive(Debug, Clone, Serialize, Default)]
pub struct ProtocolProbeReport {
    pub model: String,
    /// 通的協議；都沒有就是 `None`
    pub protocol: Option<String>,
    pub attempts: Vec<ProtocolAttempt>,
    pub requests: usize,
    pub checked_at: i64,
    /// 結論是否**確定**。
    ///
    /// 只有「成功的那個之前，每個協議都被上游明確拒絕（4xx）」才算確定。
    /// 只要有一個是 `inconclusive`（5xx／逾時／限流），就不能斷言其他協議
    /// 不支援 —— 目錄也就不該據此排除這個模型。
    pub definitive: bool,
    /// 給人看的一句話（後端組好，前端直接顯示，免得兩邊各寫一份文案）
    pub summary: String,
}

/// 依探測結果組出那句話。
fn summary_for(protocol: Option<&str>, definitive: bool, saw_inconclusive: bool) -> String {
    if !definitive && saw_inconclusive {
        return match protocol {
            Some(p) => format!(
                "實測「{p}」協議可用，但其他協議有暫時性失敗（5xx／逾時），\
                 所以**無法斷言**它只能走這一種。要確認請稍後再測一次。"
            ),
            None => "四種協議都沒通，而且其中至少一種是暫時性失敗（5xx／逾時／限流）\
                     —— 這不等於模型不支援，請稍後再測一次。"
                .into(),
        };
    }
    match protocol {
        Some("chat") => "這個模型走 OpenAI Chat 協議（/chat/completions）".into(),
        Some("responses") => "這個模型走 Responses 協議（/responses）".into(),
        Some("anthropic") => "這個模型走 Anthropic Messages 協議（/messages）；\
             Claude Code 直接可用，Codex／OpenAI 格式的客戶端由網關自動翻譯"
            .into(),
        Some("gemini") => "這個模型走 Gemini 原生協議（模型名在路徑裡：\
             /models/{model}:generateContent）；只有 Gemini 格式的客戶端可以直接用它\
             （OpenAI 系客戶端要等 OpenAI ⇄ Gemini 翻譯器）"
            .into(),
        _ => "四種協議都不通：請先確認模型名與金鑰（上游對不存在的模型通常回\
              Model is unavailable，對協議不對則回 ModelProtocolUnsupported）"
            .into(),
    }
}

/// 依協議組出請求（形狀各不相同）。
fn build_protocol_request(cfg: &ProbeCfg, protocol: &str) -> (String, serde_json::Value) {
    match protocol {
        "responses" => (
            format!("{}/responses", cfg.base),
            serde_json::json!({
                "model": cfg.actual_model,
                "input": [{"type": "message", "role": "user",
                           "content": [{"type": "input_text", "text": "hi"}]}],
                "max_output_tokens": 4,
                "stream": false,
            }),
        ),
        "anthropic" => (
            format!("{}/messages", cfg.base),
            serde_json::json!({
                "model": cfg.actual_model,
                "max_tokens": 4,
                "messages": [{"role": "user", "content": "hi"}],
            }),
        ),
        // Gemini 原生：**模型名在路徑裡**（`/models/{model}:generateContent`），
        // 內容是 `parts[]`、角色是 `user`/`model`。這是它跟另外三種最不一樣的地方。
        "gemini" => {
            // 使用者填的可能是 `models/gemini-x` 或單純 `gemini-x`，兩種都收
            let m = cfg.actual_model.trim().trim_start_matches("models/");
            (
                format!("{}/models/{m}:generateContent", cfg.base),
                serde_json::json!({
                    "contents": [{"role": "user", "parts": [{"text": "hi"}]}],
                    "generationConfig": {"maxOutputTokens": 4},
                }),
            )
        }
        _ => (
            format!("{}/chat/completions", cfg.base),
            serde_json::json!({
                "model": cfg.actual_model,
                "messages": [{"role": "user", "content": "hi"}],
                "max_tokens": 4,
                "stream": false,
            }),
        ),
    }
}

/// 送一筆協議探測。
///
/// 鑑權依**協議**決定，不是依來源：Anthropic 的 `/messages` 只認 `x-api-key`
///（實測 opencode-go 用 Bearer 回 401），Gemini 只認 `x-goog-api-key`。
fn send_protocol(cfg: &ProbeCfg, protocol: &str) -> Result<(u16, String), String> {
    let (url, body) = build_protocol_request(cfg, protocol);
    let c = client()?;
    let mut req = c
        .post(&url)
        .json(&body)
        .header("x-opencode-session", "tokentoll-probe")
        .header("User-Agent", "tokentoll-probe/0.1");
    req = if protocol == "anthropic" {
        req.header("x-api-key", &cfg.key)
            .header("anthropic-version", "2023-06-01")
    } else if protocol == "gemini" {
        req.header("x-goog-api-key", &cfg.key)
    } else {
        match cfg.scheme.as_str() {
            "goog-key" => req.header("x-goog-api-key", &cfg.key),
            "anthropic" => req
                .header("x-api-key", &cfg.key)
                .header("anthropic-version", "2023-06-01"),
            _ => req.bearer_auth(&cfg.key),
        }
    };
    let resp = req.send().map_err(|e| {
        if e.is_timeout() {
            "上游逾時".to_string()
        } else {
            format!("連接上游失敗：{e}")
        }
    })?;
    let status = resp.status().as_u16();
    Ok((status, resp.text().unwrap_or_default()))
}

/// 純網路段：逐一試四種協議，找出這個模型實際走哪一個。
pub(crate) fn probe_protocol_http(cfg: &ProbeCfg) -> ProtocolProbeReport {
    let mut rep = ProtocolProbeReport {
        model: cfg.actual_model.clone(),
        checked_at: crate::fsutil::now_ms(),
        ..Default::default()
    };
    let mut saw_inconclusive = false;
    // 來源若明確宣告了某種協議，先試它（省請求；`auto`/`mixed` 保持原順序）
    for p in probe_order(&cfg.format) {
        rep.requests += 1;
        let (status, detail) = match send_protocol(cfg, p) {
            Ok((s, text)) => (s, text.chars().take(160).collect::<String>()),
            Err(e) => (0, e),
        };
        let outcome = classify(status);
        rep.attempts.push(ProtocolAttempt {
            protocol: p.to_string(),
            status,
            outcome: outcome.to_string(),
            detail,
        });
        if outcome == "ok" {
            rep.protocol = Some(p.to_string());
            break; // 找到就停：不需要再試別的
        }
        if outcome == "inconclusive" {
            saw_inconclusive = true;
        }
    }
    // 「確定」= 在成功之前，每個協議都被上游**明確拒絕**。
    // 有任何一個是暫時性失敗 → 不能斷言其他協議不支援（見 definitive 的註解）。
    rep.definitive = rep.protocol.is_some() && !saw_inconclusive;
    rep.summary = summary_for(rep.protocol.as_deref(), rep.definitive, saw_inconclusive);
    rep
}

/// 探測用的短名 → **存進 `provider_models.protocol` 的長名**。
///
/// 探測器內部用短名（`chat` / `responses` / `anthropic` / `gemini`，因為那同時
/// 也是端點路徑的綽號），但**欄位與路由用的是長名**（`openai-chat` 等）。
///
/// 2026-10-11：原本直接把短名寫進欄位，於是 `formats_for` 拿到 `"chat"` →
/// `TargetFmt::from_db` 認不得 → `Unknown` → 走**直通**而不是翻譯，
/// Responses 客戶端會被送到 `/responses` 而不是 `/chat/completions`。
///
/// 這個函式同時用在**寫入**與**讀取**：讀取時一併容忍舊資料，所以不需要
/// 使用者做任何事。
pub(crate) fn to_stored_protocol(short: &str) -> &str {
    match short.trim() {
        "chat" => "openai-chat",
        "responses" => "openai-responses",
        // anthropic / gemini 兩邊同名
        other => other,
    }
}

/// 寫入協議記憶（不碰網路）。
///
/// 兩個地方：
/// - `provider_models.protocol`：**權威**的逐模型協議，網關路由直接讀它
///   （見 `proxy/failover.rs` 的 `formats_for`）
/// - `provider_model_protocol`（`__probe__` 鍵）：留一份帶時間戳的紀錄，
///   讓診斷頁看得出「這個結論是什麼時候得到的」
///
/// **只有結論確定時才寫**。`definitive` 為假代表「其他協議只是暫時失敗」，
/// 那時記下「這個模型只走 X」會讓目錄把它從 Codex 下架 —— 破壞性且錯誤。
///
/// 寫進去的**一律是長名**（見 [`to_stored_protocol`]）：探測器內部用短名，
/// 但欄位與路由用長名。
pub(crate) fn probe_protocol_write(
    conn: &Connection,
    provider_id: i64,
    report: &ProtocolProbeReport,
) -> Result<(), String> {
    if !report.definitive {
        return Ok(());
    }
    let Some(short) = report.protocol.as_deref() else {
        return Ok(());
    };
    let p = to_stored_protocol(short);
    conn.execute(
        "UPDATE provider_models SET protocol = ?1
         WHERE provider_id = ?2 AND actual_model = ?3",
        rusqlite::params![p, provider_id, report.model.trim()],
    )
    .map_err(|e| e.to_string())?;
    crate::trace::remember_protocol(conn, provider_id, &report.model, PROBED_PROTOCOL_KEY, p)
}

/// 讀出這個模型的有效協議：先看逐模型欄位，再看探測記憶。
///
/// 網關路由用它決定「這個模型該走哪個端點」。`None` = 未偵測
/// （呼叫端退回來源的 `api_format`；`auto` 就逐協議試）。
///
/// **回傳的一律是長名**（`openai-chat` / `openai-responses` / `anthropic` /
/// `gemini`）：2026-10-11 之前的探測把短名（`chat` / `responses`）寫進了欄位，
/// 路由因此認不得而退成直通。這裡用 [`to_stored_protocol`] 一併容忍舊資料，
/// 所以**不需要使用者做任何事**。
pub(crate) fn model_protocol(
    conn: &Connection,
    provider_id: i64,
    model: &str,
) -> Option<String> {
    let from_column: Option<String> = conn
        .query_row(
            "SELECT protocol FROM provider_models
             WHERE provider_id = ?1
               AND (lower(actual_model) = lower(?2) OR lower(display_name) = lower(?2))
             LIMIT 1",
            rusqlite::params![provider_id, model.trim()],
            |r| r.get::<_, String>(0),
        )
        .ok()
        .filter(|s: &String| !s.trim().is_empty());
    from_column
        .or_else(|| probed_protocol(conn, provider_id, model))
        .map(|p| to_stored_protocol(&p).to_string())
        .filter(|p| !p.is_empty())
}

/// 讀出實測到的協議（沒測過就 `None`）。
pub(crate) fn probed_protocol(conn: &Connection, provider_id: i64, model: &str) -> Option<String> {
    crate::trace::load_protocol(conn, provider_id, model, PROBED_PROTOCOL_KEY)
}

/// 「所有候選都被上游以 `ModelProtocolUnsupported` 拒絕」時該講什麼。
///
/// 舊訊息只說「請換模型或換來源」，把真正的原因藏起來了 —— 使用者唯一能做的
/// 是猜。
///
/// **2026-10-11 更新**：原本 `anthropic` 那一支說「Codex 到不了它」，但
/// OpenAI → Anthropic 的反向翻譯器已經實作（見 `translate/anthropic_req.rs`），
/// 那句話不再成立。現在改成講「翻譯過了卻仍被拒」，並指向最可能的原因
/// （`/messages` 普遍只認 `x-api-key`）。
///
/// `conn` 傳 `None` 代表資料庫不可用（那就只能給通用訊息）。
pub(crate) fn protocol_mismatch_message(
    conn: Option<&Connection>,
    provider_id: i64,
    model: &str,
    tried: usize,
) -> String {
    let probed = conn.and_then(|c| probed_protocol(c, provider_id, model));
    match probed.as_deref() {
        Some("anthropic") => format!(
            "模型 {model} 實測走 Anthropic Messages 協議（/messages），\
             網關已自動翻譯（OpenAI → Anthropic）卻仍被上游以「不支援本協議」拒絕。\
             最可能的原因是來源的**鑑權**：`/messages` 通常只認 `x-api-key`，\
             用 Bearer 會回 401／400。請把來源的鑑權設成 Anthropic API Key，\
             或按一次「偵測協議與思考強度」重新確認。"
        ),
        Some(p) => format!(
            "模型 {model} 在來源的所有協議下都不被接受（試過 {tried} 個候選）；\
             實測它走的是「{p}」協議。請確認來源的接口格式設定，或按一次\
             「偵測協議與思考強度」重新確認。"
        ),
        None => format!(
            "模型 {model} 在來源的所有協議下都不被接受（試過 {tried} 個候選）：\
             上游回報此模型不支援這些協議。\
             建議先按一次「偵測協議與思考強度」—— 它會一併測出這個模型實際走哪一種協議，\
             這樣才分得出是「協議不對」還是「模型名／權限有問題」。"
        ),
    }
}
