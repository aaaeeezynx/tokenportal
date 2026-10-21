//! 直連模式的同 Key 消歧與 failover 邊界。
//!
//! 直連請求的 Bearer 就是上游 Key；同一個 Key 可能掛在多個來源
//! （例如 OpenCode Go 與 Zen）。模型只登記在其中一個來源時，只允許在
//! **共用同一把 Key** 的來源間切換，不能拿別把 Key 發請求。

use super::super::*;
use axum::response::IntoResponse;

async fn echo_upstream() -> u16 {
    let fake = axum::Router::new().route(
        "/v1/chat/completions",
        axum::routing::post(
            |headers: axum::http::HeaderMap, body: axum::Json<serde_json::Value>| async move {
                let auth = headers
                    .get("authorization")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("")
                    .to_string();
                axum::Json(serde_json::json!({
                    "model": body.get("model").cloned().unwrap_or_default(),
                    "up_auth": auth,
                    "usage": {"prompt_tokens": 3, "completion_tokens": 1}
                }))
                .into_response()
            },
        ),
    );
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(l, fake).await.unwrap() });
    port
}

fn seed_direct_pair(
    db_path: &std::path::Path,
    fake_port: u16,
    a_key: &str,
    b_key: &str,
    a_has_target: bool,
    b_has_target: bool,
) {
    let conn = crate::db::open_and_ensure(db_path).unwrap();
    conn.execute(
        "INSERT INTO providers (name, app_type, api_format, base_url, api_key,
             auth_scheme, models_json, priority, enabled, created_at, updated_at)
             VALUES ('direct-a','universal','openai-chat','http://127.0.0.1:9',?1,
                     'bearer','[]',0,1,0,0),
                    ('direct-b','universal','openai-chat',?2,?3,
                     'bearer','[]',1,1,0,0)",
        rusqlite::params![a_key, format!("http://127.0.0.1:{fake_port}"), b_key],
    )
    .unwrap();
    let pa: i64 = conn
        .query_row("SELECT id FROM providers WHERE name='direct-a'", [], |r| {
            r.get(0)
        })
        .unwrap();
    let pb: i64 = conn
        .query_row("SELECT id FROM providers WHERE name='direct-b'", [], |r| {
            r.get(0)
        })
        .unwrap();
    seed_model(
        &conn,
        pa,
        if a_has_target {
            "target-model"
        } else {
            "a-model"
        },
        None,
    );
    if b_has_target {
        seed_model(&conn, pb, "target-model", None);
    }
}

async fn start_gateway(
    db_path: std::path::PathBuf,
) -> (u16, tokio::task::JoinHandle<Result<(), String>>) {
    let gw_l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = gw_l.local_addr().unwrap().port();
    let gw = tokio::spawn(serve(db_path, gw_l));
    (port, gw)
}

async fn post_model(port: u16, key: &str, model: &str) -> reqwest::Response {
    Client::new()
        .post(format!("http://127.0.0.1:{port}/v1/chat/completions"))
        .bearer_auth(key)
        .json(&serde_json::json!({"model": model}))
        .send()
        .await
        .unwrap()
}

#[tokio::test]
async fn e2e_direct_same_key_reroutes_by_model() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("direct-same.db");
    let fake_port = echo_upstream().await;
    seed_direct_pair(
        &db_path,
        fake_port,
        "shared-direct-key",
        "shared-direct-key",
        false,
        true,
    );
    let (gw_port, gw) = start_gateway(db_path).await;

    let r = post_model(gw_port, "shared-direct-key", "target-model").await;
    assert_eq!(r.status(), 200);
    let body: serde_json::Value = r.json().await.unwrap();
    assert_eq!(body["model"], "target-model");
    assert_eq!(
        body["up_auth"], "Bearer shared-direct-key",
        "應改走同 Key 的來源 B"
    );
    gw.abort();
}

#[tokio::test]
async fn e2e_direct_does_not_reroute_to_other_key() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("direct-other.db");
    let fake_port = echo_upstream().await;
    seed_direct_pair(
        &db_path,
        fake_port,
        "shared-direct-key",
        "other-direct-key",
        false,
        true,
    );
    let (gw_port, gw) = start_gateway(db_path).await;

    let r = post_model(gw_port, "shared-direct-key", "target-model").await;
    assert_eq!(r.status(), 400, "不得改走使用別把 Key 的來源 B");
    let body = r.text().await.unwrap();
    assert!(
        body.contains("direct-a"),
        "錯誤訊息應列出同 Key 來源：{body}"
    );
    assert!(body.contains("target-model"));
    gw.abort();
}

#[tokio::test]
async fn e2e_direct_failover_stays_within_same_key() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("direct-failover-other.db");
    let fake_port = echo_upstream().await;
    seed_direct_pair(
        &db_path,
        fake_port,
        "shared-direct-key",
        "other-direct-key",
        true,
        true,
    );
    let (gw_port, gw) = start_gateway(db_path).await;

    let r = post_model(gw_port, "shared-direct-key", "target-model").await;
    assert_eq!(
        r.status(),
        502,
        "綁定來源失敗時不得 failover 到別把 Key 的來源"
    );
    gw.abort();
}

#[tokio::test]
async fn e2e_direct_failover_keeps_same_key_alternative() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("direct-failover-same.db");
    let fake_port = echo_upstream().await;
    seed_direct_pair(
        &db_path,
        fake_port,
        "shared-direct-key",
        "shared-direct-key",
        true,
        true,
    );
    let (gw_port, gw) = start_gateway(db_path).await;

    let r = post_model(gw_port, "shared-direct-key", "target-model").await;
    assert_eq!(r.status(), 200);
    let body: serde_json::Value = r.json().await.unwrap();
    assert_eq!(body["up_auth"], "Bearer shared-direct-key");
    gw.abort();
}
