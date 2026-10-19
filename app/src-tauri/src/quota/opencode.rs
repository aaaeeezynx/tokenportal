//! OpenCode Go 方案額度：`GET {base_url}/usage`。

use serde_json::Value;

use super::clamp_percent;
use super::types::{QuotaWindow, SubscriptionQuota};

// ------------------------------------------------------------ OpenCode Go ---

pub(crate) fn detect_opencode_go(base_url: &str) -> bool {
    let lower = base_url.to_lowercase();
    if !lower.contains("opencode.ai") {
        return false;
    }
    // base 有兩種佈局，都是真實位址：
    //   `https://opencode.ai/zen/go/v1`（舊，只有三種協議）
    //   `https://opencode.ai/inference/go`（2026-10-11 起，官方文件記載）
    // 兩者的 `go` 都是**獨立的路径段** —— 用段比對，避免誤命中 `/google`。
    //
    // 2026-10-11 的 bug：舊版寫死比對 `"opencode.ai/zen/go"`，於是「一鍵自動
    // 偵測連線設定」把 base 換成 `/inference/go` 之後，配額卡片整張不見。
    lower.split('/').any(|s| s == "go")
}

/// 解析 `/usage` 回包：rolling/weekly/monthly 三窗口。
///
/// `resetsAt` 一律帶出來，**包括 percent 為 0 的窗口**：實測（2026-10-01 18:39）
/// 上游在 0% 時仍回一個有意義的重置點（rolling `2026-10-01T14:07:49Z`，還在 3時28分後），
/// OpenCode Go 自己的後台頁在 0% 也照樣顯示「Resets in …」。舊版「零用量就丟棄 resetsAt」
/// 只會讓卡片在剛重置完的那段時間變成「—」，看起來像壞掉。
pub(crate) fn parse_opencode_windows(body: &Value) -> Vec<QuotaWindow> {
    let mut out = vec![];
    let Some(usage) = body.get("usage") else {
        return out;
    };
    for (key, label) in [
        ("rolling", "5 小時"),
        ("weekly", "本週"),
        ("monthly", "本月"),
    ] {
        let Some(w) = usage.get(key) else { continue };
        let Some(percent) = w.get("percent").and_then(|p| p.as_f64()) else {
            continue;
        };
        let status = w.get("status").and_then(|s| s.as_str()).unwrap_or("ok");
        if status != "ok" {
            continue;
        }
        let resets_at = w
            .get("resetsAt")
            .and_then(|r| r.as_str())
            .map(|s| s.to_string());
        out.push(QuotaWindow {
            label: label.to_string(),
            percent: clamp_percent(percent),
            resets_at,
        });
    }
    out
}

/// 用量端點：`.../go/v1/usage`。
///
/// 兩種 base 佈局都成立：
/// - `/zen/go/v1`（舊）→ 去掉結尾 `/v1` 成 `/zen/go` → `/zen/go/v1/usage` ✅
/// - `/inference/go`（官方文件記載）→ `/inference/go/v1/usage` ✅
///
/// 直接接 `/usage` 在新 base 上會 404（實測 2026-10-11），所以不能靠字串相接。
pub(crate) fn usage_url(base_url: &str) -> String {
    let base = base_url.trim_end_matches('/');
    let stem = base.strip_suffix("/v1").unwrap_or(base);
    format!("{stem}/v1/usage")
}

pub(crate) fn query_opencode_go(provider_id: i64, name: &str, base_url: &str, api_key: &str) -> SubscriptionQuota {
    let key = format!("opencode-go:{provider_id}");
    let title = format!("OpenCode Go（{name}）");
    if api_key.trim().is_empty() {
        return SubscriptionQuota::no_credentials(&key, &title, "opencode-go", "該來源未填寫 API Key");
    }
    let url = usage_url(base_url);
    let client = match reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .user_agent("tokenportal/0.1 quota")
        .build()
    {
        Ok(c) => c,
        Err(e) => return SubscriptionQuota::fail(&key, &title, "opencode-go", "error", e.to_string()),
    };
    let resp = match client.get(&url).bearer_auth(api_key.trim()).send() {
        Ok(r) => r,
        Err(e) => {
            return SubscriptionQuota::fail(
                &key,
                &title,
                "opencode-go",
                "error",
                format!("連接失敗：{e}"),
            )
        }
    };
    let status = resp.status();
    if status.as_u16() == 403 {
        return SubscriptionQuota::fail(
            &key,
            &title,
            "opencode-go",
            "not_subscribed",
            "此 Key 未訂閱 OpenCode Go 方案（HTTP 403）",
        );
    }
    if status.as_u16() == 401 {
        return SubscriptionQuota::fail(
            &key,
            &title,
            "opencode-go",
            "expired",
            "Key 鑑權失敗（HTTP 401），請重新生成",
        );
    }
    if !status.is_success() {
        let snippet: String = resp.text().unwrap_or_default().chars().take(200).collect();
        return SubscriptionQuota::fail(
            &key,
            &title,
            "opencode-go",
            "error",
            format!("HTTP {status}：{}", snippet.trim()),
        );
    }
    let body: Value = match resp.json() {
        Ok(v) => v,
        Err(e) => {
            return SubscriptionQuota::fail(
                &key,
                &title,
                "opencode-go",
                "error",
                format!("解析回包失敗：{e}"),
            )
        }
    };
    let windows = parse_opencode_windows(&body);
    if windows.is_empty() {
        return SubscriptionQuota::fail(
            &key,
            &title,
            "opencode-go",
            "error",
            "回包沒有可識別的用量窗口",
        );
    }
    SubscriptionQuota::ok(&key, &title, "opencode-go", windows)
}
