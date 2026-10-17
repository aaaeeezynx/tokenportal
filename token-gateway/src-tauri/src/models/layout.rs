//! 自動偵測上游的**連線佈局**（base ＋ 逐協議路徑前綴）。
//!
//! ## 要解決的問題
//!
//! 使用者只想填「名稱 / 上游地址 / API Key」，不該還要判斷：
//!
//! - 接口格式要選 OpenAI 還是 Anthropic 還是 Gemini
//! - 逐協議路徑前綴要不要填（見 `models/prefixes.rs`）
//!
//! 這些都是**上游的事實**，不是使用者的選擇。這個模組把它們測出來、寫進設定。
//!
//! ## 判定關鍵：**路由存在 ≠ 模型支援**
//!
//! 探測時只要看回應是不是 **404**：
//!
//! - **非 404**（200、400、401…）→ 這條路由**存在**。即使上游回
//!   `ModelProtocolUnsupported`，也證明路徑是對的 —— 那只是這個模型不收。
//! - **404** → 路徑不存在，換下一個候選前綴。
//!
//! 所以**不必知道每個模型的協議**就能判斷佈局，也不需要跑很多請求。
//!
//! ## 已知主機優先
//!
//! 對已知主機（目前是 OpenCode）直接套用文件記載的佈局 —— 那是零猜測。
//! 未知主機才逐一試候選。

use rusqlite::Connection;
use serde::Serialize;
use std::time::Duration;

use super::prefixes::ProtocolPrefixes;
use super::probe::client;

/// 一次探測的上限（秒）。佈局探測是「使用者按了在等」的動作。
const TOTAL_TIMEOUT_SECS: u64 = 30;

/// 要探測的四種協議（順序固定：先試最常見的）。
const PROTOCOLS: [&str; 4] = ["openai-chat", "openai-responses", "anthropic", "gemini"];

/// 一個協議的候選前綴（依「最可能」排序）。空字串 = 沒有前綴。
fn prefix_candidates(protocol: &str) -> &'static [&'static str] {
    match protocol {
        "openai-chat" | "openai-responses" => &["", "/openai/v1"],
        "anthropic" => &["", "/anthropic/v1"],
        "gemini" => &["", "/google/v1beta", "/v1beta"],
        _ => &[""],
    }
}

/// 這個協議的端點字尾（Gemini 的模型名在路徑裡）。
pub(crate) fn suffix_for(protocol: &str, model: &str) -> String {
    let m = model.trim().trim_start_matches("models/");
    match protocol {
        "openai-chat" => "/chat/completions".to_string(),
        "openai-responses" => "/responses".to_string(),
        "anthropic" => "/messages".to_string(),
        "gemini" => format!("/models/{m}:generateContent"),
        _ => "/chat/completions".to_string(),
    }
}

/// 已知主機的佈局（host 片段 → 權威 base ＋ 四種前綴）。
///
/// 只有**真的把協議做在路徑前綴裡**的上游需要列在這裡。加新條目時請附上
/// 官方文件依據或實測紀錄。
pub(crate) struct KnownLayout {
    /// host 必須包含這個片段。
    pub(crate) host_contains: &'static str,
    /// 權威 base（`None` = 就用使用者填的）。
    pub(crate) canonical_base: Option<&'static str>,
    /// 逐協議前綴。
    pub(crate) prefixes: &'static [(&'static str, &'static str)],
}

const KNOWN: [KnownLayout; 1] = [KnownLayout {
    host_contains: "opencode.ai",
    // 官方文件（/v2/docs/console/go）記載的 base。
    // 舊的 /zen/go/v1 只有三種協議、沒有 Gemini 路由。
    canonical_base: Some("https://opencode.ai/inference/go"),
    prefixes: &[
        ("openai-chat", "/openai/v1"),
        ("openai-responses", "/openai/v1"),
        ("anthropic", "/anthropic/v1"),
        ("gemini", "/google/v1beta"),
    ],
}];

pub(crate) fn known_for(base_url: &str) -> Option<&'static KnownLayout> {
    let lower = base_url.to_ascii_lowercase();
    KNOWN.iter().find(|k| lower.contains(k.host_contains))
}

/// 探測設定。
#[derive(Debug, Clone)]
pub(crate) struct LayoutCfg {
    pub base: String,
    pub key: String,
    pub scheme: String,
    /// 用來探測 Gemini 路徑的模型名（必須是真實模型，否則可能被當成 404）。
    pub model: String,
}

/// 一個協議的探測結果。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ProtocolRoute {
    pub protocol: String,
    /// 採用的前綴（空 = 沒有前綴）
    pub prefix: String,
    /// 最後一次嘗試的 HTTP 狀態（0 = 連線失敗）
    pub status: u16,
    /// 路由存在嗎（**非 404** 就算存在 —— 見檔頭）
    pub route_exists: bool,
}

/// 佈局偵測報告。
#[derive(Debug, Clone, Serialize, Default)]
pub struct LayoutReport {
    /// 最終採用的 base
    pub base_url: String,
    /// base 是否被改成（驗證過的）權威 base
    pub base_changed: bool,
    /// 可直接存進 `providers.protocol_prefixes` 的 JSON（空 = 不需要前綴）
    pub prefixes: String,
    pub routes: Vec<ProtocolRoute>,
    /// 能用的協議（路由存在）
    pub reachable: Vec<String>,
    /// 給人看的一句話
    pub message: String,
}

/// 對一個 base 跑完整輪協議探測（只判定「路由存在」，不看模型是否支援）。
fn probe_base(cfg: &LayoutCfg, base: &str, known: Option<&KnownLayout>) -> Vec<ProtocolRoute> {
    let b = base.trim_end_matches('/');
    let Ok(c) = client() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for protocol in PROTOCOLS {
        // 已知主機：直接用它宣告的前綴（零猜測）；未知主機：逐一試候選
        let candidates: Vec<String> = match known
            .and_then(|k| k.prefixes.iter().find(|(p, _)| *p == protocol))
            .map(|(_, pre)| *pre)
        {
            Some(pre) => vec![pre.to_string()],
            None => prefix_candidates(protocol)
                .iter()
                .map(|p| p.to_string())
                .collect(),
        };
        let suffix = suffix_for(protocol, &cfg.model);
        let mut picked: Option<ProtocolRoute> = None;
        for prefix in candidates {
            let url = format!("{b}{prefix}{suffix}");
            let mut req = c
                .post(&url)
                .json(&serde_json::json!({
                    "model": cfg.model,
                    "messages": [{"role": "user", "content": "hi"}],
                    "contents": [{"role": "user", "parts": [{"text": "hi"}]}],
                    "max_tokens": 4,
                }))
                .header("x-opencode-session", "tokentoll-layout")
                .header("User-Agent", "tokentoll-layout/0.1")
                .timeout(Duration::from_secs(TOTAL_TIMEOUT_SECS));
            req = match protocol {
                "anthropic" => req
                    .header("x-api-key", &cfg.key)
                    .header("anthropic-version", "2023-06-01"),
                "gemini" => req.header("x-goog-api-key", &cfg.key),
                _ => match cfg.scheme.as_str() {
                    "goog-key" => req.header("x-goog-api-key", &cfg.key),
                    "anthropic" => req
                        .header("x-api-key", &cfg.key)
                        .header("anthropic-version", "2023-06-01"),
                    _ => req.bearer_auth(&cfg.key),
                },
            };
            let status = match req.send() {
                Ok(r) => r.status().as_u16(),
                // 連線失敗：這個候選不算「不存在」，但也沒證明存在 → 換下一個
                Err(_) => 0,
            };
            if status != 404 {
                picked = Some(ProtocolRoute {
                    protocol: protocol.to_string(),
                    prefix,
                    status,
                    route_exists: status != 0,
                });
                break;
            }
        }
        out.push(picked.unwrap_or(ProtocolRoute {
            protocol: protocol.to_string(),
            prefix: String::new(),
            status: 404,
            route_exists: false,
        }));
    }
    out
}

/// 純網路段：偵測佈局，回傳「該用哪個 base ＋ 哪些前綴」。
///
/// 已知主機優先：先試文件記載的權威 base，它若有**更多**路由可達就採用；
/// 否則保留使用者填的 base（**不無故改動設定**）。
pub(crate) fn detect_layout_http(cfg: &LayoutCfg) -> LayoutReport {
    let known = known_for(&cfg.base);
    let mut report = LayoutReport {
        base_url: cfg.base.trim_end_matches('/').to_string(),
        ..Default::default()
    };

    let mut routes = probe_base(cfg, &report.base_url, None);
    let mut reachable = routes.iter().filter(|r| r.route_exists).count();

    // 已知主機：權威 base 若可達更多協議就切換（**驗過才換**）
    if let Some(k) = known {
        if let Some(canon) = k.canonical_base {
            if canon.trim_end_matches('/') != report.base_url {
                let alt = probe_base(cfg, canon, Some(k));
                let alt_reachable = alt.iter().filter(|r| r.route_exists).count();
                if alt_reachable > reachable {
                    report.base_changed = true;
                    report.base_url = canon.trim_end_matches('/').to_string();
                    routes = alt;
                    reachable = alt_reachable;
                }
            }
        }
    }

    // 組出要存的前綴 JSON（只留「這個協議真的需要前綴」的）
    let mut saved = ProtocolPrefixes::default();
    for r in &routes {
        if !r.route_exists || r.prefix.is_empty() {
            continue;
        }
        match r.protocol.as_str() {
            "openai-chat" => saved.openai_chat = r.prefix.clone(),
            "openai-responses" => saved.openai_responses = r.prefix.clone(),
            "anthropic" => saved.anthropic = r.prefix.clone(),
            "gemini" => saved.gemini = r.prefix.clone(),
            _ => {}
        }
    }
    report.prefixes = if saved == ProtocolPrefixes::default() {
        String::new()
    } else {
        serde_json::to_string(&saved).unwrap_or_default()
    };
    report.reachable = routes
        .iter()
        .filter(|r| r.route_exists)
        .map(|r| r.protocol.clone())
        .collect();

    let base_note = if report.base_changed {
        format!(
            "上游地址已換成官方記載的 {}（原本那個少了部分協議的路由）；",
            report.base_url
        )
    } else {
        String::new()
    };
    report.message = if reachable == 0 {
        format!(
            "{base_note}四種協議的路由都不存在 —— 請確認上游地址與 API Key（沒有改動任何設定）"
        )
    } else {
        format!(
            "{base_note}可用的協議：{}（共 {reachable}/4）",
            report.reachable.join(" / ")
        )
    };
    report.routes = routes;
    report
}

/// 短鎖段：讀出偵測需要的連線資訊（含一個可用的模型名）。
pub(crate) fn layout_read_cfg(conn: &Connection, provider_id: i64) -> Result<LayoutCfg, String> {
    let (base, key, scheme) = conn
        .query_row(
            "SELECT base_url, api_key, auth_scheme FROM providers WHERE id=?1",
            [provider_id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            },
        )
        .map_err(|_| "找不到這個來源".to_string())?;
    if base.trim().is_empty() {
        return Err("請先填寫上游地址".to_string());
    }
    if key.trim().is_empty() {
        return Err("請先填寫 API Key".to_string());
    }
    // 探測要靠一個**真實**模型：假的模型名可能被上游回 404，被誤判成「路由不存在」。
    let model: Option<String> = conn
        .query_row(
            "SELECT actual_model FROM provider_models
             WHERE provider_id=?1 AND actual_model <> '' ORDER BY ord, id LIMIT 1",
            [provider_id],
            |r| r.get(0),
        )
        .ok()
        .or_else(|| {
            conn.query_row(
                "SELECT models_json FROM model_catalogs WHERE provider_id=?1",
                [provider_id],
                |r| r.get::<_, String>(0),
            )
            .ok()
            .and_then(|j| serde_json::from_str::<Vec<String>>(&j).ok())
            .and_then(|v| v.into_iter().next())
        });
    let Some(model) = model.filter(|m| !m.trim().is_empty()) else {
        return Err("請先「取得模型清單」或新增一個模型，偵測需要用真實模型名".to_string());
    };
    Ok(LayoutCfg {
        base: base.trim().to_string(),
        key,
        scheme,
        model,
    })
}

/// 短鎖段：把偵測結果寫回（只寫「有路由可達」的部分；全都不可達就什麼都不動）。
pub(crate) fn layout_write(
    conn: &Connection,
    provider_id: i64,
    report: &LayoutReport,
) -> Result<(), String> {
    if report.reachable.is_empty() {
        return Ok(()); // 不改動 —— 絕不弄壞一個本來能用的來源
    }
    conn.execute(
        "UPDATE providers SET base_url=?1, protocol_prefixes=?2, api_format='auto',
         updated_at=?3 WHERE id=?4",
        rusqlite::params![
            report.base_url,
            report.prefixes,
            crate::fsutil::now_ms(),
            provider_id
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}
