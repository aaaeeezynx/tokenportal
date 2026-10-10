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

/// 三種協議的候選（依「最可能」排序：先試來源宣告的那種）。
const PROTOCOLS: [&str; 3] = ["chat", "responses", "anthropic"];

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
            None => "三種協議都沒通，而且其中至少一種是暫時性失敗（5xx／逾時／限流）\
                     —— 這不等於模型不支援，請稍後再測一次。"
                .into(),
        };
    }
    match protocol {
        Some("chat") => "這個模型走 OpenAI Chat 協議（/chat/completions）".into(),
        Some("responses") => "這個模型走 Responses 協議（/responses）".into(),
        Some("anthropic") => "這個模型只走 Anthropic Messages 協議（/messages）——\
             Codex／OpenAI 格式的客戶端到不了它，請改用 Claude Code，\
             或把它放到一個「接口格式 = anthropic」的來源"
            .into(),
        _ => "三種協議都不通：請先確認模型名與金鑰（上游對不存在的模型通常回\
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

/// 送一筆協議探測。Anthropic 端點只認 `x-api-key`（實測 Bearer 回 401）。
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

/// 純網路段：逐一試三種協議，找出這個模型實際走哪一個。
pub(crate) fn probe_protocol_http(cfg: &ProbeCfg) -> ProtocolProbeReport {
    let mut rep = ProtocolProbeReport {
        model: cfg.actual_model.clone(),
        checked_at: crate::fsutil::now_ms(),
        ..Default::default()
    };
    let mut saw_inconclusive = false;
    for p in PROTOCOLS {
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

/// 寫入協議記憶（不碰網路）。
///
/// **只有結論確定時才寫**。`definitive` 為假代表「其他協議只是暫時失敗」，
/// 那時記下「這個模型只走 X」會讓目錄把它從 Codex 下架 —— 破壞性且錯誤。
pub(crate) fn probe_protocol_write(
    conn: &Connection,
    provider_id: i64,
    report: &ProtocolProbeReport,
) -> Result<(), String> {
    if !report.definitive {
        return Ok(());
    }
    let Some(p) = report.protocol.as_deref() else {
        return Ok(());
    };
    crate::trace::remember_protocol(conn, provider_id, &report.model, PROBED_PROTOCOL_KEY, p)
}

/// 讀出實測到的協議（沒測過就 `None`）。
pub(crate) fn probed_protocol(conn: &Connection, provider_id: i64, model: &str) -> Option<String> {
    crate::trace::load_protocol(conn, provider_id, model, PROBED_PROTOCOL_KEY)
}

/// 這個模型是不是「Codex 到不了」的（實測只走 Anthropic）。
///
/// 目錄生成用它把這種模型**排除**：提供一個永遠會失敗的模型比不提供更糟
/// —— 使用者選了只會拿到「此模型不支援本協議」，而原因完全看不出來。
pub(crate) fn is_unreachable_for_openai_clients(
    conn: &Connection,
    provider_id: i64,
    model: &str,
) -> bool {
    probed_protocol(conn, provider_id, model).as_deref() == Some("anthropic")
}

/// 「所有候選都被上游以 `ModelProtocolUnsupported` 拒絕」時該講什麼。
///
/// 舊訊息只說「請換模型或換來源」，把真正的原因藏起來了 —— 使用者唯一能做的
/// 是猜。2026-10-10 的實例：opencode-go 的 `claude-haiku-5-5` 只吃 Anthropic
/// Messages，而網關**沒有** OpenAI → Anthropic 的反向翻譯，所以在 Codex 底下
/// 它永遠不可能成功。這種事要直接講，並給出真的可行的下一步。
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
            "模型 {model} 只走 Anthropic Messages 協議（/messages），\
             但這個客戶端用的是 OpenAI 格式（Codex 走 Responses）。\
             網關沒有「OpenAI → Anthropic」的反向翻譯，所以它在此路徑下無法使用。\
             做法：改用 Claude Code（Anthropic 入站可以直接用它），\
             或為它另開一個「接口格式 = anthropic、鑑權 = anthropic」的來源。"
        ),
        Some(p) => format!(
            "模型 {model} 在來源的所有協議下都不被接受（試過 {tried} 個候選）；\
             實測它走的是「{p}」協議。請確認來源的接口格式設定，或按一次\
             「偵測思考強度」重新確認。"
        ),
        None => format!(
            "模型 {model} 在來源的所有協議下都不被接受（試過 {tried} 個候選）：\
             上游回報此模型不支援這些協議。\
             建議先按一次「偵測思考強度」—— 它會一併測出這個模型實際走哪一種協議，\
             這樣才分得出是「協議不對」還是「模型名／權限有問題」。"
        ),
    }
}
