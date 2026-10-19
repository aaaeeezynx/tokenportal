//! 剝離與直通修復（B1／B2 的規則）
//!
//! 由 `proxy/tests.rs` 拆分而來（原檔 2,025 行）。測試**內容與斷言一字
//! 未改**，只搬位置；對外仍經由 `proxy::tests::` 這個路徑被呼叫。

    // 前置由父模組 `proxy::tests` 的 `pub(super) use` 再匯出提供（含 `SseUsage`
    // 與 `E_*` 常數）；`use super::*;` 會把它們一併帶進來。這裡**不可**再寫
    // `use super::matrix::…` —— 在子模組裡 `super` 是 `proxy::tests`，而那裡
    // 的 `matrix` 指的是測試子模組本身。
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn strip_unsupported_tools_keeps_function() {
        let body = serde_json::json!({
            "model": "deepseek-v4-flash",
            "tools": [
                {"type": "function", "function": {"name": "web_search", "parameters": {}}},
                {"type": "custom", "name": "computer_use", "parameters": {}},
                {"type": "custom", "name": "shell", "parameters": {}}
            ],
            "tool_choice": {"type": "function", "function": {"name": "web_search"}},
            "messages": [{"role": "user", "content": "hi"}]
        });
        let bytes = serde_json::to_vec(&body).unwrap();
        let out = strip_unsupported_tools(&bytes).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
        let tools = v["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["type"], "function");
        // tool_choice 仍存在（引用的 tool 還在）
        assert!(v.get("tool_choice").is_some());
    }


    #[test]
    fn strip_unsupported_tools_removes_all() {
        let body = serde_json::json!({
            "model": "m",
            "tools": [
                {"type": "custom", "name": "a"},
                {"type": "shell", "name": "b"}
            ],
            "tool_choice": "required",
            "messages": [{"role": "user", "content": "hi"}]
        });
        let bytes = serde_json::to_vec(&body).unwrap();
        let out = strip_unsupported_tools(&bytes).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert!(v.get("tools").is_none());
        assert!(v.get("tool_choice").is_none());
    }


    #[test]
    fn strip_unsupported_tools_no_tools() {
        let body = serde_json::json!({"model": "m", "messages": []});
        let bytes = serde_json::to_vec(&body).unwrap();
        // 沒有 tools 欄位，原樣返回
        assert!(strip_unsupported_tools(&bytes).is_none());
    }


    #[test]
    fn passthrough_chat_sanitize_repairs_and_noops() {        // 孤兒 tool_calls（舊中斷 session）→ 補佔位
        let broken = br#"{"model":"m","messages":[
            {"role":"user","content":"hi"},
            {"role":"assistant","content":null,"tool_calls":[{"id":"c1","type":"function","function":{"name":"f","arguments":"{}"}}]}
        ]}"#;
        let out = sanitize_passthrough_chat_body(broken, InFmt::OpenAi, false).expect("應修復");
        let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
        let msgs = v["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[2]["role"], "tool");
        assert_eq!(msgs[2]["tool_call_id"], "c1");
        // 有效歷史一字不動
        let ok_body = br#"{"model":"m","messages":[{"role":"user","content":"hi"}]}"#;
        assert!(sanitize_passthrough_chat_body(ok_body, InFmt::OpenAi, false).is_none());
        // 非 chat / 已翻譯不碰
        assert!(sanitize_passthrough_chat_body(broken, InFmt::Anthropic, false).is_none());
        assert!(sanitize_passthrough_chat_body(broken, InFmt::OpenAi, true).is_none());
        assert!(sanitize_passthrough_chat_body(b"not json", InFmt::OpenAi, false).is_none());
    }


    #[test]
    fn strip_encrypted_content_removes_nested_occurrences() {
        // 第三方 responses 端點會以「encrypted_content was not issued to this
        // caller」400 拒收，故直通前必須剝離所有副本（含巢狀）。
        let body = br#"{"model":"m","input":[
            {"type":"reasoning","encrypted_content":"SECRET","summary":[]},
            {"type":"message","role":"user","content":[{"type":"input_text","text":"hi"}],
             "meta":{"encrypted_content":"ALSO_SECRET"}}
        ],"store":false}"#;
        let out = strip_encrypted_content(body).expect("應命中並剝離");
        let s = String::from_utf8(out).unwrap();
        assert!(!s.contains("encrypted_content"), "仍有殘留: {s}");
        assert!(!s.contains("SECRET"), "簽章值未移除: {s}");
        // 其餘欄位一字不動
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["model"], "m");
        assert_eq!(v["store"], false);
        assert_eq!(v["input"].as_array().unwrap().len(), 2);
        assert_eq!(v["input"][0]["type"], "reasoning");
        assert_eq!(v["input"][1]["content"][0]["text"], "hi");
    }


    #[test]
    fn strip_encrypted_content_is_noop_when_absent() {
        // 未命中必須回 None（呼叫端據此判斷「原文一字不動」）
        assert!(strip_encrypted_content(br#"{"model":"m","input":"hi"}"#).is_none());
    }

    /// `store: false` 的判斷（2026-10-10 的 bug 靠它決定要不要剝簽章）。
    #[test]
    fn is_stateless_detects_store_false_only() {
        assert!(is_stateless(br#"{"model":"m","store":false}"#));
        assert!(is_stateless(br#"{"model":"m","store": false}"#));
        assert!(is_stateless(br#"{"model":"m", "store" :false ,"x":1}"#));
        assert!(!is_stateless(br#"{"model":"m","store":true}"#));
        assert!(!is_stateless(br#"{"model":"m"}"#));
        // 值不是布林、或只是別處出現 "store" 字樣，都不算
        assert!(!is_stateless(br#"{"model":"m","store":"false"}"#));
        assert!(!is_stateless(br#"{"note":"store"}"#));
        // 註：實作是全文掃描，巢狀的 store 也會命中。Responses 請求的 store
        // 一定是頂層，而誤判的方向是「保留簽章」—— 比剝掉安全，故不特別排除。
    }

    /// ★ 迴歸：`store: false` 時**不可**剝掉推理簽章。
    ///
    /// 那時上游不保存回應，推理的重播完全依賴 `encrypted_content`；
    /// 剝掉會換來 `Referenced reasoning item 'rs_…' was not found or has expired.`
    #[test]
    fn stateless_request_keeps_encrypted_content() {
        let dir = tempfile::tempdir().unwrap();
        let conn = crate::db::open_and_ensure(&dir.path().join("s.db")).unwrap();
        let body = br#"{"model":"m","store":false,"input":[
            {"type":"reasoning","id":"rs_1","encrypted_content":"SIG","summary":[]}
        ]}"#;
        let out = strip_for_upstream(&conn, 7, body.to_vec(), InFmt::Responses, false);
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains("SIG"), "無狀態請求必須保留簽章：{s}");
    }

    /// 非無狀態（上游有保存）→ 照舊剝掉，涵蓋原本那個 ChatGPT 簽章問題。
    #[test]
    fn stateful_request_still_strips_encrypted_content() {
        let dir = tempfile::tempdir().unwrap();
        let conn = crate::db::open_and_ensure(&dir.path().join("s2.db")).unwrap();
        let body = br#"{"model":"m","store":true,"input":[
            {"type":"reasoning","id":"rs_1","encrypted_content":"SIG","summary":[]}
        ]}"#;
        let out = strip_for_upstream(&conn, 7, body.to_vec(), InFmt::Responses, false);
        let s = String::from_utf8(out).unwrap();
        assert!(!s.contains("SIG"), "有保存時仍該剝掉：{s}");
    }

    /// 已學到「這個來源就是拒收簽章」→ 連無狀態請求也要剝。
    #[test]
    fn learned_rejection_overrides_stateless_keep() {
        let dir = tempfile::tempdir().unwrap();
        let conn = crate::db::open_and_ensure(&dir.path().join("s3.db")).unwrap();
        crate::trace::remember_stripped(&conn, 7, ENCRYPTED_CONTENT_FIELD).unwrap();
        let body = br#"{"model":"m","store":false,"input":[
            {"type":"reasoning","id":"rs_1","encrypted_content":"SIG","summary":[]}
        ]}"#;
        let out = strip_for_upstream(&conn, 7, body.to_vec(), InFmt::Responses, false);
        let s = String::from_utf8(out).unwrap();
        assert!(!s.contains("SIG"), "學到拒收後就該剝：{s}");
    }

    /// 只認這個片語，別種 400 不該被誤判成「要剝簽章」。
    #[test]
    fn encrypted_content_rejection_phrase_is_specific() {
        assert!(is_encrypted_content_rejected(
            "reasoning `encrypted_content` was not issued to this caller"
        ));
        assert!(!is_encrypted_content_rejected("Invalid request parameters"));
        assert!(!is_encrypted_content_rejected(
            "Referenced reasoning item 'rs_1' was not found or has expired."
        ));
    }


    #[test]
    fn strip_encrypted_content_handles_arrays_and_deep_nesting() {
        let body =
            br#"{"a":[[{"encrypted_content":"x"}]],"b":{"c":{"d":{"encrypted_content":"y"}}}}"#;
        let out = strip_encrypted_content(body).expect("應命中");
        let s = String::from_utf8(out).unwrap();
        assert!(!s.contains("encrypted_content"), "深層殘留: {s}");
    }


    #[test]
    fn strip_encrypted_content_leaves_similar_keys_alone() {
        // 只剝離精確鍵名，不得誤傷其他含相似字串的鍵
        let body = br#"{"my_encrypted_content_x":1,"encrypted_content":"z"}"#;
        let out = strip_encrypted_content(body).expect("應命中");
        let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert!(v.get("encrypted_content").is_none());
        assert_eq!(v["my_encrypted_content_x"], 1, "相似鍵名被誤刪");
    }


    #[test]
    fn strip_encrypted_content_ignores_non_json() {
        assert!(strip_encrypted_content(b"not json").is_none());
        assert!(strip_encrypted_content(b"").is_none());
    }
