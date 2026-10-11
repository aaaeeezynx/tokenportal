//! Anthropic 形狀進出的端到端測試。

    use super::super::*;

    /// 翻譯 e2e：Anthropic 進 → 假 OpenAI 出 → 客戶端收到 Anthropic 形狀 + 按實際模型記帳；
    /// 反向（OpenAI → Anthropic 渠道）明確 400。
    #[tokio::test]
    async fn e2e_anthropic_to_openai() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("tr.db");
        {
            let conn = crate::db::open_and_ensure(&db_path).unwrap();
            conn.execute(
                "INSERT INTO providers (name, app_type, api_format, base_url, api_key, auth_scheme, models_json, priority, enabled, created_at, updated_at)
                 VALUES ('tr','universal','openai-chat','PLACEHOLDER','up-key','bearer','[]',0,1,0,0)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO providers (name, app_type, api_format, base_url, api_key, auth_scheme, models_json, priority, enabled, created_at, updated_at)
                 VALUES ('an','universal','anthropic','http://127.0.0.1:1','k','anthropic','[]',0,1,0,0)",
                [],
            )
            .unwrap();
            let pid: i64 = conn
                .query_row("SELECT id FROM providers WHERE name='tr'", [], |r| r.get(0))
                .unwrap();
            conn.execute(
                "INSERT INTO provider_models (provider_id, display_name, actual_model, reasoning, ord, enabled)
                 VALUES (?1, 'claude-sonnet-4-6', 'deepseek-v4-flash', 'unset', 0, 1)",
                [pid],
            )
            .unwrap();
        }
        // 假 OpenAI 上游：斷言收到轉換後的請求
        let fake = axum::Router::new().route(
            "/chat/completions",
            axum::routing::post(|body: axum::Json<serde_json::Value>| async move {
                assert_eq!(body["model"], "deepseek-v4-flash");
                assert_eq!(body["messages"][0]["role"], "system");
                assert_eq!(body["messages"][1]["content"], "hi");
                axum::Json(serde_json::json!({
                    "id": "chatcmpl-t1", "model": "deepseek-v4-flash",
                    "choices": [{"message": {"content": "ok"}, "finish_reason": "stop"}],
                    "usage": {"prompt_tokens": 8, "completion_tokens": 2}
                }))
                .into_response()
            }),
        );
        let fake_l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let fake_port = fake_l.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(fake_l, fake).await.unwrap() });

        // 假 **Anthropic** 上游：斷言收到翻譯後的 Anthropic 形狀（反向那條）
        let fake_an = axum::Router::new().route(
            "/messages",
            axum::routing::post(|body: axum::Json<serde_json::Value>| async move {
                assert_eq!(body["model"], "x", "模型名應原樣映射過去");
                assert_eq!(body["messages"][0]["content"][0]["text"], "hi");
                // 使用者指定：思考強度直接映射到延伸思考
                assert_eq!(body["thinking"]["type"], "enabled");
                assert_eq!(body["thinking"]["budget_tokens"], 16384, "high → 16384");
                assert!(
                    body["max_tokens"].as_i64().unwrap() > 16384,
                    "budget_tokens 必須嚴格小於 max_tokens：{body:?}"
                );
                axum::Json(serde_json::json!({
                    "id": "msg_1", "type": "message", "role": "assistant",
                    "model": "x",
                    "content": [{"type": "text", "text": "from anthropic"}],
                    "stop_reason": "end_turn",
                    "usage": {"input_tokens": 3, "output_tokens": 4}
                }))
                .into_response()
            }),
        );
        let an_l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let an_port = an_l.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(an_l, fake_an).await.unwrap() });

        let (secret, an_secret) = {
            let conn = crate::db::open_and_ensure(&db_path).unwrap();
            conn.execute(
                "UPDATE providers SET base_url=?1 WHERE name='tr'",
                [format!("http://127.0.0.1:{fake_port}")],
            )
            .unwrap();
            conn.execute(
                "UPDATE providers SET base_url=?1 WHERE name='an'",
                [format!("http://127.0.0.1:{an_port}")],
            )
            .unwrap();
            let mk = |name: &str| {
                let pid: i64 = conn
                    .query_row("SELECT id FROM providers WHERE name=?1", [name], |r| {
                        r.get(0)
                    })
                    .unwrap();
                keys::create_key(
                    &conn,
                    &keys::KeyInput {
                        name: name.into(),
                        provider_id: Some(pid),
                        quota_tokens: -1,
                        rate_limit_qpm: 0,
                        allowed_models: vec![],
                        allowed_apps: vec![],
                        expires_at: None,
                    },
                )
                .unwrap()
                .secret
            };
            (mk("tr"), mk("an"))
        };
        let gw_l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let gw_port = gw_l.local_addr().unwrap().port();
        let gw = tokio::spawn(serve(db_path.clone(), gw_l));
        let http = Client::new();

        // 1) Anthropic 進 → Anthropic 出
        let r = http
            .post(format!("http://127.0.0.1:{gw_port}/v1/messages"))
            .bearer_auth(&secret)
            .header("anthropic-version", "2023-06-01")
            .json(&serde_json::json!({
                "model": "claude-sonnet-4-6",
                "max_tokens": 16,
                "system": "sys",
                "messages": [{"role": "user", "content": "hi"}]
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        let body: serde_json::Value = r.json().await.unwrap();
        assert_eq!(body["type"], "message");
        assert_eq!(body["model"], "claude-sonnet-4-6", "回顯請求模型");
        assert_eq!(body["content"][0]["text"], "ok");
        assert_eq!(body["usage"]["input_tokens"], 8);
        assert_eq!(body["usage"]["output_tokens"], 2);
        // 按實際模型記帳
        let conn = crate::db::open_and_ensure(&db_path).unwrap();
        let (m, itok, otok): (String, i64, i64) = conn
            .query_row(
                "SELECT model_raw, in_tok, out_tok FROM request_logs ORDER BY id DESC LIMIT 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!((m.as_str(), itok, otok), ("deepseek-v4-flash", 8, 2));

        // 2) 反向（OpenAI 請求打 Anthropic 渠道）：**2026-10-10 起會翻譯**，
        //    不再是「反向轉換尚未實現」的 400。假上游在 /messages 上斷言形狀。
        let r = http
            .post(format!("http://127.0.0.1:{gw_port}/v1/chat/completions"))
            .bearer_auth(&an_secret)
            .json(&serde_json::json!({
                "model": "x",
                "messages": [{"role": "user", "content": "hi"}],
                "reasoning_effort": "high"
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 200, "反向翻譯器已實作，應成功而非 400");
        let body: serde_json::Value = r.json().await.unwrap();
        assert_eq!(body["object"], "chat.completion");
        assert_eq!(body["choices"][0]["message"]["content"], "from anthropic");
        assert_eq!(body["usage"]["prompt_tokens"], 3);
        assert_eq!(body["usage"]["completion_tokens"], 4);

        gw.abort();
    }
