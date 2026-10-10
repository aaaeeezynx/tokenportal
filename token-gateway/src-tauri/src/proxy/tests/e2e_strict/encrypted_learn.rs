//! 被動學習：上游拒收推理簽章時剝掉重試，並記住「這個來源要剝」。
//!
//! 對應 2026-10-10 的另一半修正：`store: false` 的請求我們預設**保留**
//! `encrypted_content`（那是上游重播推理的唯一依據），但若這個來源真的拒收
//! 簽章（`was not issued to this caller`），就得剝掉重試並記住 —— 下次連
//! 無狀態請求也直接剝，不必再撞一次。

// super 取本檔（e2e_strict）的共用 helper，super::super 取 proxy::tests 的前置
use super::*;
use super::super::*;

/// 送出一個帶推理簽章的 `store: false` 請求。
///
/// 回傳 `(回應狀態, 上游每次收到的 body 是否含 encrypted_content, db 路徑,
/// 來源 id, TempDir)`。**TempDir 要一起回傳**：它一被 drop，目錄（連同資料庫）
/// 就消失，呼叫端之後便開不了連線。
#[allow(clippy::type_complexity)]
async fn run_case(
    upstream_rejects_signature: bool,
) -> (u16, Vec<bool>, std::path::PathBuf, i64, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("enc.db");
    let (pid, secret) = {
        let conn = crate::db::open_and_ensure(&db_path).unwrap();
        let pid = seed_provider(&conn, "enc-src");
        seed_model(&conn, pid, "enc-model", None);
        let secret = seed_key(&conn, pid, "enc");
        (pid, secret)
    };

    let seen: std::sync::Arc<std::sync::Mutex<Vec<bool>>> =
        std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen_fake = seen.clone();
    let fake = axum::Router::new().fallback(
        move |axum::Json(body): axum::Json<serde_json::Value>| {
            let seen = seen_fake.clone();
            async move {
                let has_sig = body.to_string().contains("encrypted_content");
                seen.lock().unwrap().push(has_sig);
                if has_sig && upstream_rejects_signature {
                    (
                        axum::http::StatusCode::BAD_REQUEST,
                        r#"{"error":{"message":"reasoning `encrypted_content` was not issued to this caller"}}"#
                            .to_string(),
                    )
                } else {
                    (
                        axum::http::StatusCode::OK,
                        r#"{"id":"r1","object":"response","status":"completed",
                            "output":[{"type":"message","role":"assistant",
                                       "content":[{"type":"output_text","text":"hi"}]}],
                            "usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}}"#
                            .to_string(),
                    )
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
            "UPDATE providers SET base_url=?1, api_format='openai-responses' WHERE name='enc-src'",
            [format!("http://127.0.0.1:{fake_port}")],
        )
        .unwrap();
    }
    let gw_l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let gw_port = gw_l.local_addr().unwrap().port();
    tokio::spawn(serve(db_path.clone(), gw_l));
    let http = Client::new();

    let r = http
        .post(format!("http://127.0.0.1:{gw_port}/v1/responses"))
        .bearer_auth(&secret)
        .json(&serde_json::json!({
            "model": "enc-model",
            "store": false,
            "input": [
                {"type": "message", "role": "user",
                 "content": [{"type": "input_text", "text": "hi"}]},
                {"type": "reasoning", "id": "rs_1", "summary": [],
                 "encrypted_content": "SIGNATURE"}
            ]
        }))
        .send()
        .await
        .unwrap();
    let status = r.status().as_u16();
    let seq = seen.lock().unwrap().clone();
    (status, seq, db_path, pid, dir)
}

/// ★ 迴歸（2026-10-10）：`store: false` 時簽章**必須原樣送到上游**。
///
/// 舊版無條件剝掉它，於是上游只剩一個查不到的 ID，回
/// `Referenced reasoning item 'rs_…' was not found or has expired.`
#[tokio::test]
async fn e2e_stateless_request_keeps_encrypted_content() {
    let (status, seq, _db, _pid, _dir) = run_case(false).await;
    assert_eq!(status, 200);
    assert_eq!(seq, vec![true], "上游必須收到 encrypted_content");
}

/// 上游真的拒收簽章 → 剝掉重試、成功，並**記住這個來源要剝**。
#[tokio::test]
async fn e2e_rejected_signature_is_stripped_retried_and_remembered() {
    let (status, seq, db_path, pid, _dir) = run_case(true).await;
    assert_eq!(status, 200, "剝掉簽章後應該成功");
    assert_eq!(seq, vec![true, false], "先帶簽章被拒、再剝掉重試：{seq:?}");

    // ★ 核心：記住了，下次不必再撞一次
    let conn = open_conn(&db_path).unwrap();
    let learned = crate::trace::load_stripped(&conn, pid);
    assert!(
        learned.iter().any(|f| f == ENCRYPTED_CONTENT_FIELD),
        "應記下這個來源拒收簽章，實得 {learned:?}"
    );
}
