//! 端到端：**Gemini 客戶端在 `auto` 來源上用 Gemini 模型**（2026-10-11）。
//!
//! 這是「逐模型協議」對 Gemini 也成立的那一半。修之前：`formats_for` 把 Gemini
//! 入站落進 `_ => "openai-chat"`，而 `(Gemini, OpenAiChat)` 是矩陣明定的
//! `E_GEMINI_IN_ONLY`；偏偏 `plan_attempts` 對第一個候選是**無條件**排入的，
//! 所以 Gemini 客戶端在 `auto` 來源上第一個候選就被擋下 —— 即使模型是
//! Gemini 原生的也一樣。
//!
//! 注意這**不含**「OpenAI 系客戶端用 Gemini 模型」：那要等 OpenAI ⇄ Gemini
//! 翻譯器（下一段工作）。

use super::*;

/// 只認 Gemini 路徑的假上游；記下收到的路徑。
fn fake_gemini() -> (u16, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
    let seen: std::sync::Arc<std::sync::Mutex<Vec<String>>> =
        std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen_srv = seen.clone();
    let app = axum::Router::new().fallback(
        move |uri: axum::http::Uri| {
            let seen = seen_srv.clone();
            async move {
                let path = uri.path().to_string();
                seen.lock().unwrap().push(path.clone());
                if path.contains(":generateContent") {
                    (
                        axum::http::StatusCode::OK,
                        r#"{"candidates":[{"content":{"parts":[{"text":"hi from gemini"}]},
                            "finishReason":"STOP"}],
                            "usageMetadata":{"promptTokenCount":3,"candidatesTokenCount":4}}"#
                            .to_string(),
                    )
                } else {
                    // 非 Gemini 路徑一律 404（真實的 Gemini 端點就是這樣）
                    (axum::http::StatusCode::NOT_FOUND, "not found".to_string())
                }
            }
        },
    );
    let (tx, rx) = std::sync::mpsc::channel::<u16>();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async move {
            let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            tx.send(l.local_addr().unwrap().port()).unwrap();
            axum::serve(l, app).await.unwrap();
        });
    });
    (rx.recv().unwrap(), seen)
}

/// 一個 `auto` 來源，兩個模型：一個標成 Gemini、一個是普通的 chat 模型。
async fn setup() -> (
    u16,
    String,
    std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    tempfile::TempDir,
) {
    let (up, seen) = fake_gemini();
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("gem.db");
    let secret = {
        let conn = crate::db::open_and_ensure(&db_path).unwrap();
        let pid = seed_provider(&conn, "gem-src");
        conn.execute(
            "UPDATE providers SET api_format='auto', auth_scheme='goog-key', base_url=?1
             WHERE name='gem-src'",
            [format!("http://127.0.0.1:{up}")],
        )
        .unwrap();
        seed_model(&conn, pid, "gemini-ish", None);
        seed_model(&conn, pid, "chatty", None);
        conn.execute(
            "UPDATE provider_models SET protocol='gemini' WHERE actual_model='gemini-ish'",
            [],
        )
        .unwrap();
        conn.execute(
            "UPDATE provider_models SET protocol='openai-chat' WHERE actual_model='chatty'",
            [],
        )
        .unwrap();
        seed_key(&conn, pid, "gem")
    };
    let gw_l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let gw_port = gw_l.local_addr().unwrap().port();
    tokio::spawn(serve(db_path, gw_l));
    (gw_port, secret, seen, dir)
}

/// ★ 核心：Gemini 原生請求 ＋ `auto` 來源 → **直通 Gemini 端點**，一次就中。
#[tokio::test]
async fn auto_source_passes_gemini_request_through() {
    let (gw, secret, seen, _dir) = setup().await;
    let r = Client::new()
        .post(format!(
            "http://127.0.0.1:{gw}/v1beta/models/gemini-ish:generateContent"
        ))
        .header("x-goog-api-key", &secret)
        .json(&serde_json::json!({
            "contents": [{"role": "user", "parts": [{"text": "hi"}]}]
        }))
        .send()
        .await
        .unwrap();
    let status = r.status();
    let body = r.text().await.unwrap();
    assert_eq!(status, 200, "Gemini 請求應該直通成功；body={body}");

    let s = seen.lock().unwrap().clone();
    assert_eq!(s.len(), 1, "不該有失敗往返（修之前第一個候選就被擋下）：{s:?}");
    assert!(
        s[0].contains(":generateContent"),
        "應打到 Gemini 端點：{s:?}"
    );
    // 回包原樣透傳（Gemini 進、Gemini 出）
    assert!(body.contains("hi from gemini"), "{body}");
}

/// 同一個來源的另一個模型走 chat：Gemini 客戶端打它 → 矩陣明定的
/// `E_GEMINI_IN_ONLY`（沒有 Gemini → OpenAI 的翻譯器），但**不該**無限重試。
#[tokio::test]
async fn gemini_client_gets_a_clear_error_for_a_chat_model() {
    let (gw, secret, seen, _dir) = setup().await;
    let r = Client::new()
        .post(format!(
            "http://127.0.0.1:{gw}/v1beta/models/chatty:generateContent"
        ))
        .header("x-goog-api-key", &secret)
        .json(&serde_json::json!({
            "contents": [{"role": "user", "parts": [{"text": "hi"}]}]
        }))
        .send()
        .await
        .unwrap();
    assert!(
        !r.status().is_success(),
        "chat-only 的模型對 Gemini 客戶端本來就不可用"
    );
    // 關鍵：一個請求都不該送到上游（矩陣在送出前就擋下了）
    let s = seen.lock().unwrap().clone();
    assert!(s.is_empty(), "不該送出任何上游請求：{s:?}");
}
