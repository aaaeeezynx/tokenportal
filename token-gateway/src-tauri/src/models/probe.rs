//! 思考強度探測：逐一實測「這個模型收不收這個 `reasoning_effort`」。
//!
//! ## 為什麼要直打上游、不經本地網關
//!
//! 走網關會經過翻譯器的強度轉發邏輯，所以量到的會是「翻譯器願意送什麼」而不是
//! 「上游接受什麼」。2026-10-08 就是這樣量錯的：`ultra` 在網關的 Responses 路徑
//! 上回 200，但那是因為它被丟掉、上游根本沒收到；直打上游才看到它其實是 400。
//!
//! ## 基準請求（沒有它就會誤判）
//!
//! 每個模型先送一筆**不帶 effort** 的請求。只有基準 2xx 才繼續測候選 ——
//! 否則一個金鑰過期或模型名打錯的模型，會被誤判成「所有強度都不支援」，
//! 然後把使用者的設定清空。
//!
//! ## 請求形狀照渠道格式走
//!
//! 探測要重現**網關實際會送出的那個欄位**：
//! - `openai-chat`／`mixed`：`/chat/completions` ＋ 攤平的 `reasoning_effort`
//!   （網關的翻譯路徑就是產生這個）
//! - `openai-responses`：`/responses` ＋ `reasoning: {effort}`
//!   （直通路徑原樣轉發 Responses body）
//! - `anthropic`／`gemini`：機制不同（`thinking`／`thinkingConfig`），**不支援探測**

use rusqlite::Connection;
use serde::Serialize;
use std::time::Duration;

use super::reasoning::{record, CANDIDATE_LEVELS, LEVEL_NONE};

/// 連線階段上限（秒）：使用者在等一個按鈕，不該等滿整體逾時。
const CONNECT_TIMEOUT_SECS: u64 = 6;
/// 整體上限（秒）：實測正常模型 1–12 秒回應。
const TOTAL_TIMEOUT_SECS: u64 = 30;

/// 探測設定（短鎖段讀出，網路階段不再碰 DB）。
#[derive(Debug, Clone)]
pub(crate) struct ProbeCfg {
    pub base: String,
    pub key: String,
    pub scheme: String,
    /// 上游實際模型名（送出去的 `model` 欄位）
    pub actual_model: String,
    pub format: String,
    /// 逐協議路徑前綴的原始 JSON（空 = 用預設）。見 `models/prefixes.rs`。
    pub prefixes: String,
}

/// 一筆探測的結果。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LevelOutcome {
    pub level: String,
    pub supported: bool,
    /// 上游回應摘要（給人看；失敗時特別有用）
    pub detail: String,
}

/// 整輪探測的報告。
#[derive(Debug, Clone, Serialize, Default)]
pub struct ReasoningProbeReport {
    pub model: String,
    /// 基準請求是否通過（沒通過就不會測任何候選）
    pub baseline_ok: bool,
    /// 基準失敗 / 整輪中止的原因（人話）
    pub aborted: Option<String>,
    /// 明確支援的強度
    pub supported: Vec<String>,
    /// 明確不支援的強度
    pub unsupported: Vec<String>,
    /// 拿不到明確答案的（`429`／`5xx`／逾時／其他 4xx）——**不寫入記憶**
    pub inconclusive: Vec<LevelOutcome>,
    /// 這一輪實際發出的上游請求數
    pub requests: usize,
    pub checked_at: i64,
}

// ------------------------------------------------------------ 短鎖段 ---

/// 讀出探測需要的連線資訊（不碰網路）。
pub(crate) fn probe_read_cfg(
    conn: &Connection,
    provider_id: i64,
    model_id: i64,
) -> Result<ProbeCfg, String> {
    let (base, key, scheme, format, actual_model, prefixes) = conn
        .query_row(
            "SELECT p.base_url, p.api_key, p.auth_scheme, p.api_format, m.actual_model,
                    p.protocol_prefixes
             FROM provider_models m JOIN providers p ON p.id = m.provider_id
             WHERE m.id = ?1 AND m.provider_id = ?2",
            rusqlite::params![model_id, provider_id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                ))
            },
        )
        .map_err(|_| "找不到這個模型（可能已被刪除）".to_string())?;
    if base.trim().is_empty() {
        return Err("這個來源還沒填上游地址".to_string());
    }
    if key.trim().is_empty() {
        return Err("這個來源還沒填 API Key，無法探測".to_string());
    }
    if actual_model.trim().is_empty() {
        return Err("這個模型沒有填「實際請求模型」".to_string());
    }
    match format.as_str() {
        "openai-chat" | "mixed" | "openai-responses" => {}
        other => {
            return Err(format!(
                "「{other}」渠道的思考強度不是用 reasoning_effort 控制的\
                 （Anthropic 用 thinking、Gemini 用 thinkingConfig），無法用這個方式探測"
            ))
        }
    }
    Ok(ProbeCfg {
        base: base.trim_end_matches('/').to_string(),
        key,
        scheme,
        actual_model,
        format,
        prefixes,
    })
}

/// 把探測結果寫進記憶**並帶入模型設定**（不碰網路）。
///
/// 兩件事刻意分開：
/// - **記憶**（`provider_model_reasoning`）只寫「有明確答案」的：`inconclusive`
///   的那些**刻意不寫** —— 一次 429 或上游 5xx 不該被記成「這個強度不支援」。
/// - **使用者的宣告**（`provider_models.reasoning`）則改寫成實測支援集合，
///   這就是「自動取得並帶入」。被動學習不會做這件事（見 `proxy/retry.rs`），
///   只有使用者明確按下偵測才會動他的設定。
pub(crate) fn probe_write(
    conn: &Connection,
    provider_id: i64,
    report: &ReasoningProbeReport,
) -> rusqlite::Result<()> {
    if !report.baseline_ok {
        return Ok(());
    }
    let now = crate::fsutil::now_ms();
    record(conn, provider_id, &report.model, LEVEL_NONE, true, "probe", now)?;
    for lv in &report.supported {
        record(conn, provider_id, &report.model, lv, true, "probe", now)?;
    }
    for lv in &report.unsupported {
        record(conn, provider_id, &report.model, lv, false, "probe", now)?;
    }
    // 帶入設定：實測支援集合即為宣告值（空就是空 —— 搭配 has_facts，
    // 目錄會宣告成空陣列而不是退回模板並集）
    let declared = report.supported.join(",");
    conn.execute(
        "UPDATE provider_models SET reasoning = ?1
         WHERE provider_id = ?2 AND actual_model = ?3",
        rusqlite::params![declared, provider_id, report.model.trim()],
    )?;
    Ok(())
}

// ------------------------------------------------------------ 網路段 ---

pub(super) fn client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS))
        .timeout(Duration::from_secs(TOTAL_TIMEOUT_SECS))
        .build()
        .map_err(|e| e.to_string())
}

/// 依渠道格式組出請求（`level = None` 代表基準請求）。
fn build_request(
    cfg: &ProbeCfg,
    level: Option<&str>,
) -> (String, serde_json::Value) {
    if cfg.format == "openai-responses" {
        let mut body = serde_json::json!({
            "model": cfg.actual_model,
            "input": [{"type": "message", "role": "user",
                       "content": [{"type": "input_text", "text": "hi"}]}],
            "max_output_tokens": 4,
            "stream": false,
        });
        if let Some(lv) = level {
            body["reasoning"] = serde_json::json!({"effort": lv});
        }
        (format!("{}/responses", cfg.base), body)
    } else {
        // openai-chat / mixed：翻譯路徑產生的就是攤平的 reasoning_effort
        let mut body = serde_json::json!({
            "model": cfg.actual_model,
            "messages": [{"role": "user", "content": "hi"}],
            "max_tokens": 4,
            "stream": false,
        });
        if let Some(lv) = level {
            body["reasoning_effort"] = serde_json::json!(lv);
        }
        (format!("{}/chat/completions", cfg.base), body)
    }
}

fn send(cfg: &ProbeCfg, body: &serde_json::Value) -> Result<(u16, String), String> {
    let url = build_request(cfg, None).0;
    let c = client()?;
    let mut req = c
        .post(&url)
        .json(body)
        // 網關會把客戶端的這兩個標頭原樣轉發上游，所以探測也必須帶 ——
        // 否則量到的不是「上游收不收這個強度」，而是「上游收不收這個請求」。
        //
        // 2026-10-08 由真機測試抓到：opencode-go 少了 `x-opencode-session`
        // 會直接回 400 `MissingSessionID`（基準就失敗，整輪誤判成中止）；
        // 而 Cloudflare 對泛用的程式庫 UA 會回 403。
        .header("x-opencode-session", "tokentoll-probe")
        .header("User-Agent", "tokentoll-probe/0.1");
    req = match cfg.scheme.as_str() {
        "goog-key" => req.header("x-goog-api-key", &cfg.key),
        "anthropic" => req
            .header("x-api-key", &cfg.key)
            .header("anthropic-version", "2023-06-01"),
        _ => req.bearer_auth(&cfg.key),
    };
    let resp = req.send().map_err(|e| {
        if e.is_timeout() {
            "上游逾時".to_string()
        } else {
            format!("連接上游失敗：{e}")
        }
    })?;
    let status = resp.status().as_u16();
    let text = resp.text().unwrap_or_default();
    Ok((status, text))
}

/// 基準失敗 → 這不是「強度不支援」的問題，整輪中止。
fn baseline_abort_reason(status: u16, body: &str) -> String {
    let head: String = body.chars().take(160).collect();
    match status {
        401 | 403 => format!("金鑰或權限有問題（HTTP {status}）：{head}"),
        404 | 405 => format!("端點或模型名不對（HTTP {status}）：{head}"),
        s if (500..600).contains(&s) => format!("上游故障（HTTP {s}）：{head}"),
        429 => format!("被上游限流（HTTP 429）：{head}"),
        s => format!("上游回非預期狀態（HTTP {s}）：{head}"),
    }
}

/// 純網路段：跑完整輪探測（不碰 DB，可安全放在鎖外）。
pub(crate) fn probe_model_http(cfg: &ProbeCfg) -> ReasoningProbeReport {
    let mut rep = ReasoningProbeReport {
        model: cfg.actual_model.clone(),
        checked_at: crate::fsutil::now_ms(),
        ..Default::default()
    };
    let (_, baseline) = build_request(cfg, None);

    // ---- 基準 ----
    match send(cfg, &baseline) {
        Err(e) => {
            rep.aborted = Some(e);
            return rep;
        }
        Ok((status, body)) => {
            rep.requests += 1;
            if !(200..300).contains(&status) {
                rep.aborted = Some(baseline_abort_reason(status, &body));
                return rep;
            }
        }
    }
    rep.baseline_ok = true;

    // ---- 逐一測候選 ----
    for lv in CANDIDATE_LEVELS {
        let (_, body) = build_request(cfg, Some(lv));
        rep.requests += 1;
        match send(cfg, &body) {
            Ok((status, text)) => {
                let head: String = text.chars().take(160).collect();
                if (200..300).contains(&status) {
                    rep.supported.push(lv.to_string());
                } else if status == 400 {
                    // 基準已經通過，唯一的差別就是這個欄位 → 判定不支援。
                    // 保留原文，讓使用者能看到上游到底怎麼說。
                    rep.unsupported.push(lv.to_string());
                    if rep.inconclusive.is_empty() {
                        rep.inconclusive.push(LevelOutcome {
                            level: format!("{lv}（樣本）"),
                            supported: false,
                            detail: head,
                        });
                    }
                } else {
                    // 429／5xx／其他 4xx：不是強度的問題，不寫入記憶
                    rep.inconclusive.push(LevelOutcome {
                        level: lv.to_string(),
                        supported: false,
                        detail: format!("HTTP {status}：{head}"),
                    });
                }
            }
            Err(e) => rep.inconclusive.push(LevelOutcome {
                level: lv.to_string(),
                supported: false,
                detail: e,
            }),
        }
    }
    rep
}
