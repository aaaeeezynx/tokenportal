//! 空回包現場傾印的測試。

use super::super::dump::dump_upstream_failure;

/// 寫出來的檔案要含請求體與上游回應，而且路徑可以回報給人。
#[test]
fn writes_the_request_and_upstream_bodies() {
    let tmp = std::env::temp_dir().join(format!("tg-dump-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let db = tmp.join("app.db");

    let req = br#"{"model":"gemini-4","contents":[]}"#;
    let path = dump_upstream_failure(&db, req, b"").expect("應寫出傾印檔");
    assert!(path.exists(), "檔案不存在: {}", path.display());
    assert_eq!(path.parent().unwrap(), tmp.join("dumps"));

    let v: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(v["request_length"], req.len());
    assert_eq!(v["upstream_length"], 0);
    assert!(v["request_body"].as_str().unwrap().contains("gemini-4"));
    assert_eq!(v["upstream_body"], "");

    let _ = std::fs::remove_dir_all(&tmp);
}

/// 超過上限就不寫（不要把整段長對話倒進磁碟）。
#[test]
fn skips_oversized_request_bodies() {
    let tmp = std::env::temp_dir().join(format!("tg-dump-big-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let db = tmp.join("app.db");

    let huge = vec![b'x'; 600 * 1024];
    assert!(dump_upstream_failure(&db, &huge, b"").is_none());

    let _ = std::fs::remove_dir_all(&tmp);
}

/// 只留最新 20 份 —— 人要看的是最近一次。
#[test]
fn keeps_only_the_newest_dumps() {
    let tmp = std::env::temp_dir().join(format!("tg-dump-prune-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let db = tmp.join("app.db");
    let dir = tmp.join("dumps");
    std::fs::create_dir_all(&dir).unwrap();

    // 先手動塞 25 個假的舊檔（名字格式與正式的一致）
    for i in 0..25 {
        std::fs::write(
            dir.join(format!("upstream-400-{:013}-10.json", 1_000_000_000_000_i64 + i)),
            b"{}",
        )
        .unwrap();
    }
    // 再寫一份新的，觸發 prune
    dump_upstream_failure(&db, b"{}", b"").expect("應寫出");

    let n = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_str()
                .is_some_and(|n| n.starts_with("upstream-400-"))
        })
        .count();
    assert_eq!(n, 20, "應只留 20 份，實際 {n}");

    let _ = std::fs::remove_dir_all(&tmp);
}
