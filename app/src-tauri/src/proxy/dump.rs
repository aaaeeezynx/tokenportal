//! 上游 400「空回包」的現場傾印。
//!
//! 有些上游（2026-10-12 的 OpenCode Go Gemini 路由就是）在拒收請求時回
//! **400 而且 body 是空的** —— 一個字都沒有，錯誤訊息、欄位名、什麼都沒給。
//! 這時候唯一能查的就是**我們送出去的原文**，但 `proxy_trace` 只存長度與
//! 指紋（不存明文，隱私考量），所以事後完全無法重現。
//!
//! 這個模組就補那一段：把送出的請求體與上游回應寫成一個 JSON 檔，
//! 路徑附加在錯誤訊息與 trace note 上，人可以直接打開來看。

use std::path::{Path, PathBuf};

/// 單檔上限：超過就不寫（避免把整段長對話倒進磁碟）。
const MAX_BODY: usize = 512 * 1024;
/// 保留最近幾份（人只需要最近的一次）。
const KEEP: usize = 20;

/// 把一次失敗的往返寫成 `<app_data>/dumps/upstream-400-<ts>-<len>.json`。
///
/// 回傳寫出的路徑（給 trace note 與客戶端訊息引用）；任何一步失敗就回 `None`
/// —— 傾印是輔助，**絕不能讓它影響代理本體**。
pub(super) fn dump_upstream_failure(
    db_path: &Path,
    request_body: &[u8],
    upstream_body: &[u8],
) -> Option<PathBuf> {
    if request_body.len() > MAX_BODY {
        return None;
    }
    let dir = db_path.parent()?.join("dumps");
    std::fs::create_dir_all(&dir).ok()?;
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_millis();
    let path = dir.join(format!(
        "upstream-400-{ts}-{}.json",
        request_body.len()
    ));
    let payload = serde_json::json!({
        "ts_ms": ts,
        "request_length": request_body.len(),
        "upstream_length": upstream_body.len(),
        // 存成字串（不是二進位）：人要看得懂，而且請求體本來就是 JSON
        "request_body": String::from_utf8_lossy(request_body),
        "upstream_body": String::from_utf8_lossy(upstream_body),
    });
    let bytes = serde_json::to_vec_pretty(&payload).ok()?;
    std::fs::write(&path, bytes).ok()?;
    prune(&dir);
    Some(path)
}

/// 只留最新的 [`KEEP`] 份。名字裡的 `<ts>` 是毫秒，字典序即時間序。
fn prune(dir: &Path) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut names: Vec<PathBuf> = rd
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("upstream-400-"))
        })
        .collect();
    if names.len() <= KEEP {
        return;
    }
    names.sort();
    // names 由舊到新，砍掉前面多出來的
    for p in &names[..names.len() - KEEP] {
        let _ = std::fs::remove_file(p);
    }
}
