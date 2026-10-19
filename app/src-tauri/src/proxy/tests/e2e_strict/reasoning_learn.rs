//! 被動學習：上游拒收 `reasoning_effort` 時記下該強度並重試。
//!
//! 這一條是「零額外請求」的那半：不必先按偵測，真實踩到就學會。

// super 取本檔（e2e_strict）的共用 helper，super::super 取 proxy::tests 的前置
use super::*;
use super::super::*;

/// 假上游對「帶 reasoning_effort」的請求回泛用 400，不帶就正常回應。
/// 斷言：① 重試成功（使用者拿到 200）② 該強度被記成不支援。
#[tokio::test]
async fn e2e_unsupported_reasoning_effort_is_learned_and_retried() {
    use axum::response::IntoResponse;

    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("ro.db");
    let (pid, secret) = {
        let conn = crate::db::open_and_ensure(&db_path).unwrap();
        let pid = seed_provider(&conn, "ro-src");
        seed_model(&conn, pid, "ro-model", Some("low,medium,high,xhigh"));
        let secret = seed_key(&conn, pid, "ro");
        (pid, secret)
    };

    let seen: std::sync::Arc<std::sync::Mutex<Vec<String>>> =
        std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen_fake = seen.clone();
    let fake = axum::Router::new().fallback(
        move |axum::Json(body): axum::Json<serde_json::Value>| {
            let seen = seen_fake.clone();
            async move {
                let has = body.get("reasoning_effort").is_some();
                seen.lock().unwrap().push(if has { "with".into() } else { "without".into() });
                if has {
                    // 上游對不認識的強度回的就是這句（沒有欄位名可解析）
                    (
                        axum::http::StatusCode::BAD_REQUEST,
                        r#"{"error":{"message":"Invalid request parameters"}}"#.to_string(),
                    )
                        .into_response()
                } else {
                    axum::Json(serde_json::json!({
                        "id": "c1", "object": "chat.completion", "model": "ro-model",
                        "choices": [{"index": 0, "finish_reason": "stop",
                            "message": {"role": "assistant", "content": "hi"}}],
                        "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}
                    }))
                    .into_response()
                }
            }
        },
    );
    let fake_l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let fake_port = fake_l.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(fake_l, fake).await.unwrap() });
    {
        let conn = crate::db::open_and_ensure(&db_path).unwrap();
        conn.execute(
            "UPDATE providers SET base_url=?1 WHERE name='ro-src'",
            [format!("http://127.0.0.1:{fake_port}")],
        )
        .unwrap();
    }
    let gw_l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let gw_port = gw_l.local_addr().unwrap().port();
    let gw = tokio::spawn(serve(db_path.clone(), gw_l));
    let http = Client::new();

    // 走 chat 直通（不翻譯）→ body 原樣送出，effort 一定到得了上游
    let r = http
        .post(format!("http://127.0.0.1:{gw_port}/v1/chat/completions"))
        .bearer_auth(&secret)
        .json(&serde_json::json!({
            "model": "ro-model",
            "messages": [{"role": "user", "content": "hi"}],
            "reasoning_effort": "xhigh"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(r.status(), 200, "拿掉不支援的強度後應該成功");
    let seq = seen.lock().unwrap().clone();
    assert_eq!(
        seq,
        vec!["with".to_string(), "without".to_string()],
        "應該先帶 effort 被拒、再拿掉重試：{seq:?}"
    );

    // ★ 核心：這個強度被記成「不支援」
    let conn = open_conn(&db_path).unwrap();
    let uns = crate::models::known_unsupported(&conn, pid, "ro-model");
    assert_eq!(uns, vec!["xhigh".to_string()], "應記下 xhigh 不支援");
    // 而且沒有把其他強度一起誤殺
    assert!(crate::models::known_supported(&conn, pid, "ro-model").is_empty());
    gw.abort();
}
