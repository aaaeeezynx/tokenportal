//! 真機驗證：協議探測對真實 opencode-go 的判斷。
//!
//! 2026-10-10 手動量測的結論：`claude-haiku-5-5` 只走 Anthropic Messages
//! （`/messages` + `x-api-key`），`/chat/completions` 回
//! `ModelProtocolUnsupported`；而 `deepseek-v4.1-flash` 走 chat。
//! 探測器必須自己得出同一組答案。
//!
//! `cargo test --offline --lib -- --ignored live_probe_protocol --nocapture`

use super::*;

/// ★ 真機：**對現有來源按一次「自動偵測」，看它能不能自己升級佈局**。
///
/// 使用者的 `opencode-go` 來源原本是舊佈局（`/zen/go/v1`），只有三種協議、
/// 沒有 Gemini 路由。偵測會：
///
/// 1. 先量舊 base → 應該只有 3 條路由
/// 2. 再量官方記載的 base → 應該有 4 條
/// 3. 4 > 3 → **切換**（只有在驗證過之後才換）
/// 4. 寫入前綴、把接口格式設成 `auto`
///
/// **會改動真實資料庫的那一筆來源**（那正是 UI 按鈕做的事）。
///
/// `cargo test --offline --lib -- --ignored live_detect_layout --nocapture`
#[test]
#[ignore = "live: 需真實來源與金鑰，且會改寫該來源的 base／前綴（等同按一次 UI 按鈕）"]
fn live_detect_layout_upgrades_the_existing_opencode_source() {
    let db = dirs::data_dir()
        .expect("data_dir")
        .join("com.tokencounter.gateway")
        .join("app.db");
    let conn = crate::db::open_and_ensure(&db).expect("打開真實資料庫");

    let pid: i64 = match conn.query_row(
        "SELECT id FROM providers WHERE base_url LIKE '%opencode.ai%' ORDER BY id LIMIT 1",
        [],
        |r| r.get(0),
    ) {
        Ok(v) => v,
        Err(_) => {
            println!("（跳過：找不到 opencode 來源）");
            return;
        }
    };
    let before: (String, String, String) = conn
        .query_row(
            "SELECT base_url, api_format, protocol_prefixes FROM providers WHERE id=?1",
            [pid],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    println!(
        "偵測前：base={} fmt={} prefixes={:?}",
        before.0, before.1, before.2
    );

    let cfg = layout_read_cfg(&conn, pid).expect("讀取偵測設定");
    let rep = detect_layout_http(&cfg);
    println!("{}", rep.message);
    for r in &rep.routes {
        println!(
            "    {:<18} prefix={:<14} HTTP {} route={}",
            r.protocol, r.prefix, r.status, r.route_exists
        );
    }
    assert_eq!(rep.reachable.len(), 4, "官方 base 應四種協議都通：{rep:?}");
    assert!(rep.base_changed, "舊佈局只有三種協議，應該會升級 base");

    layout_write(&conn, pid, &rep).expect("寫入偵測結果");
    let after: (String, String, String) = conn
        .query_row(
            "SELECT base_url, api_format, protocol_prefixes FROM providers WHERE id=?1",
            [pid],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    println!(
        "偵測後：base={} fmt={} prefixes={:?}",
        after.0, after.1, after.2
    );
    assert_eq!(after.0, "https://opencode.ai/inference/go");
    assert_eq!(after.1, "auto");
    assert!(after.2.contains("/google/v1beta"), "{}", after.2);
}

/// ★ 真機：**探測器自己要能透過「多路徑形狀」找到 Gemini**。
///
/// 2026-10-11 的誤判：opencode-go 的 `gemini-4-argon-contributor` 在站上可用，
/// 但探測把標準形 `/models/{m}:generateContent` 寫死，於是打不到真正的
/// `/google/v1beta/models/{m}:generateContent`，誤報「四種協議都不通」。
///
/// 這裡用**文件記載的 base**（`https://opencode.ai/inference/go`）跑一次，
/// 斷言探測器自己找到 `gemini` —— 而且是在第二個形狀才中。
///
/// `cargo test --offline --lib -- --ignored live_probe_gemini --nocapture`
#[test]
#[ignore = "live: 需要 opencode-go 金鑰（會對上游發幾筆極小請求）"]
fn live_probe_finds_gemini_through_the_protocol_prefixed_path() {
    let db = dirs::data_dir()
        .expect("data_dir")
        .join("com.tokencounter.gateway")
        .join("app.db");
    let conn = crate::db::open_and_ensure(&db).expect("打開真實資料庫");
    let key: String = match conn.query_row("SELECT api_key FROM providers WHERE id = 22", [], |r| {
        r.get(0)
    }) {
        Ok(k) => k,
        Err(_) => {
            println!("（跳過：找不到 source 22）");
            return;
        }
    };

    let cfg = ProbeCfg {
        // 文件記載的 base（不是 /zen/go/v1）
        base: "https://opencode.ai/inference/go".into(),
        key,
        scheme: "bearer".into(),
        actual_model: "gemini-4-argon-contributor".into(),
        protocol: "gemini".into(),
        prefixes: String::new(),
    };
    let rep = probe_protocol_http(&cfg);
    println!(
        "protocol={:?} requests={} definitive={}\n  {}",
        rep.protocol, rep.requests, rep.definitive, rep.summary
    );
    for a in &rep.attempts {
        println!("    {} -> HTTP {} ({})", a.protocol, a.status, a.outcome);
    }
    assert_eq!(
        rep.protocol.as_deref(),
        Some("gemini"),
        "探測器應該透過 /google/v1beta 那個形狀找到它；{rep:?}"
    );
    // 第一個形狀（標準形）會 404，第二個才中 → 2 筆（format=gemini 讓它排第一）
    assert_eq!(rep.requests, 2, "應是標準形 404 → 協議前綴形 200：{rep:?}");
}

/// ★ 真機：**重新生成目錄並確認 Anthropic-only 的模型在裡面**。
///
/// 2026-10-11 的實例：`claude-haiku-5-5` 因為「Codex 到不了 Anthropic-only
/// 模型」的排除條件而沒被寫進目錄，使用者在 Codex 的模型選單裡看不到它
/// —— 但反向翻譯器其實已經讓它可用了。排除條件已移除，這條測試釘住結果。
///
/// **會覆寫真實的 `catalogs/codex-<pid>.json`**（那正是 UI 按「套用」做的事）。
///
/// `cargo test --offline --lib -- --ignored live_catalog_includes --nocapture`
#[test]
#[ignore = "live: 會覆寫真實的 Codex 目錄檔（等同在 UI 按一次套用）"]
fn live_catalog_includes_anthropic_only_models() {
    let app_data = dirs::data_dir()
        .expect("data_dir")
        .join("com.tokencounter.gateway");
    let db = app_data.join("app.db");
    let conn = crate::db::open_and_ensure(&db).expect("打開真實資料庫");
    let home = dirs::home_dir().expect("home_dir");

    let pid: Option<i64> = conn
        .query_row(
            "SELECT m.provider_id FROM provider_models m
             JOIN providers p ON p.id = m.provider_id
             WHERE m.actual_model = 'claude-haiku-5-5' AND p.enabled = 1 LIMIT 1",
            [],
            |r| r.get(0),
        )
        .ok();
    let Some(pid) = pid else {
        println!("（跳過：資料庫裡沒有 claude-haiku-5-5）");
        return;
    };

    // `union_others = false` 與 UI 的預設一致（`catalog_union ?? false`）：
    // 只列當前來源的模型，不要把其他來源的也併進來。
    let dest = crate::models::codex_catalog_generate(&conn, &home, &app_data, pid, false)
        .expect("生成目錄");
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&dest).unwrap()).unwrap();
    let slugs: Vec<String> = v["models"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["slug"].as_str().unwrap_or("").to_string())
        .collect();
    println!("目錄 {} 個模型：", slugs.len());
    for s in &slugs {
        println!("  {s}");
    }
    assert!(
        slugs.iter().any(|s| s == "claude-haiku-5-5"),
        "Anthropic-only 的模型必須在目錄裡，否則 Codex 選單看不到它"
    );
    assert!(slugs.len() > 5, "目錄不該只剩一兩個模型：{slugs:?}");
}

/// ★ 真機驗收：**同一個來源、Claude Code 用得了 Anthropic-only 的模型**。
///
/// 這是使用者真正要的那條路。舊版下，`mixed` 來源的 Anthropic 請求一律被翻成
/// chat 送到 `/chat/completions`，於是 `claude-haiku-5-5` 從 Claude Code 也失敗
/// —— 只能把同一個訂閱拆成兩個來源（金鑰重複、統計被切碎）。
///
/// 流程：① 探測並寫入逐模型協議 ② 用 Anthropic 形狀打本地網關 ③ 應 200。
///
/// **會寫入真實資料庫的 `provider_models.protocol`**（那正是 UI 按鈕做的事），
/// 也會對上游發幾筆極小請求。
///
/// `cargo test --offline --lib -- --ignored live_claude_code --nocapture`
#[test]
#[ignore = "live: 需真實來源與金鑰，且需本地網關在跑（會改動 DB 的 protocol 欄）"]
fn live_claude_code_can_use_an_anthropic_only_model_on_a_mixed_source() {
    let db = dirs::data_dir()
        .expect("data_dir")
        .join("com.tokencounter.gateway")
        .join("app.db");
    let conn = crate::db::open_and_ensure(&db).expect("打開真實資料庫");

    let row: Option<(i64, i64)> = conn
        .query_row(
            "SELECT m.provider_id, m.id FROM provider_models m
             JOIN providers p ON p.id = m.provider_id
             WHERE m.actual_model = 'claude-haiku-5-5' AND p.enabled = 1 LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .ok();
    let Some((pid, mid)) = row else {
        println!("（跳過：資料庫裡沒有 claude-haiku-5-5）");
        return;
    };
    let fmt: String = conn
        .query_row("SELECT api_format FROM providers WHERE id=?1", [pid], |r| {
            r.get(0)
        })
        .unwrap();
    let key: String = conn
        .query_row(
            "SELECT key_plain FROM local_keys WHERE provider_id=?1 AND enabled=1 LIMIT 1",
            [pid],
            |r| r.get(0),
        )
        .expect("找不到本地 Key");

    // ① 探測並寫入逐模型協議
    let cfg = probe_read_cfg(&conn, pid, mid).expect("讀取探測設定");
    let rep = probe_protocol_http(&cfg);
    println!(
        "探測結果：{:?}（definitive={}）",
        rep.protocol, rep.definitive
    );
    assert_eq!(rep.protocol.as_deref(), Some("anthropic"), "{rep:?}");
    assert!(rep.definitive, "必須是明確結論才寫入：{rep:?}");
    probe_protocol_write(&conn, pid, &rep).expect("寫入協議");
    println!(
        "已寫入 provider_models.protocol = {:?}（來源 api_format 仍是 {fmt}）",
        model_protocol(&conn, pid, "claude-haiku-5-5")
    );

    // ② 用 Anthropic 形狀打本地網關
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(90))
        .build()
        .unwrap();
    let r = client
        .post("http://127.0.0.1:15722/v1/messages")
        .header("content-type", "application/json")
        .header("x-api-key", &key)
        .header("anthropic-version", "2023-06-01")
        .json(&serde_json::json!({
            "model": "claude-haiku-5-5",
            "max_tokens": 16,
            "messages": [{"role": "user", "content": "say hi"}]
        }))
        .send()
        .expect("本地網關沒在跑？");
    let status = r.status().as_u16();
    let body = r.text().unwrap_or_default();
    println!("網關回應：HTTP {status}  {}", &body[..body.len().min(200)]);
    assert_eq!(
        status, 200,
        "Claude Code 形狀的請求應該走 /messages 直通成功；body={body}"
    );
}

/// ★★ 最終驗收：**Codex（Responses）用得了 Anthropic-only 的模型**。
///
/// 這是反向翻譯器存在的理由。舊版這一格是
/// `Err(E_REVERSE_UNSUPPORTED)`（「反向轉換尚未實現」），Codex 永遠到不了
/// `claude-haiku-5-5`。
///
/// 路徑：Responses 請求 → `responses_to_openai` → `chat_to_anthropic`
///       → `POST /messages` → `anthropic_event_to_chat_chunks`
///       → `chat_chunk_to_responses` → Codex 收到 Responses 事件。
///
/// **需本地網關在跑**，且該模型的逐模型協議已是 `anthropic`
/// （上一條 live 測試會寫入）。
///
/// `cargo test --offline --lib -- --ignored live_codex_uses --nocapture`
#[test]
#[ignore = "live: 需真實來源與金鑰，且需本地網關在跑（會對上游發一筆請求）"]
fn live_codex_uses_an_anthropic_only_model() {
    let db = dirs::data_dir()
        .expect("data_dir")
        .join("com.tokencounter.gateway")
        .join("app.db");
    let conn = crate::db::open_and_ensure(&db).expect("打開真實資料庫");

    let row: Option<(i64, String)> = conn
        .query_row(
            "SELECT m.provider_id, m.protocol FROM provider_models m
             JOIN providers p ON p.id = m.provider_id
             WHERE m.actual_model = 'claude-haiku-5-5' AND p.enabled = 1 LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .ok();
    let Some((pid, proto)) = row else {
        println!("（跳過：資料庫裡沒有 claude-haiku-5-5）");
        return;
    };
    if proto != "anthropic" {
        println!("（跳過：protocol={proto:?}，請先跑 live_claude_code 那條寫入）");
        return;
    }
    let key: String = conn
        .query_row(
            "SELECT key_plain FROM local_keys WHERE provider_id=?1 AND enabled=1 LIMIT 1",
            [pid],
            |r| r.get(0),
        )
        .expect("找不到本地 Key");

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .unwrap();
    // Codex 的實際形狀：Responses ＋ 串流
    let r = client
        .post("http://127.0.0.1:15722/v1/responses")
        .header("content-type", "application/json")
        .bearer_auth(&key)
        .json(&serde_json::json!({
            "model": "claude-haiku-5-5",
            "instructions": "Answer in one short sentence.",
            "input": [{"type": "message", "role": "user",
                       "content": [{"type": "input_text", "text": "say hi"}]}],
            "max_output_tokens": 64,
            "stream": true,
            "store": false
        }))
        .send()
        .expect("本地網關沒在跑？");
    let status = r.status().as_u16();
    let body = r.text().unwrap_or_default();
    println!("HTTP {status}");
    println!("{}", &body[..body.len().min(700)]);
    assert_eq!(status, 200, "Codex 形狀的請求應該成功；body={body}");
    // 客戶端必須拿到 **Responses** 事件，而不是 Anthropic 的
    assert!(body.contains("response.created"), "應有 response.created");
    assert!(
        body.contains("response.completed") || body.contains("response.output_text.delta"),
        "應有 Responses 的輸出事件"
    );
    assert!(
        !body.contains("content_block_delta") && !body.contains("message_start"),
        "不該把 Anthropic 的事件名漏給客戶端"
    );
}

#[test]
#[ignore = "live: 需要真實來源與金鑰（每個模型最多 3 筆極小請求）"]
fn live_probe_protocol_on_opencode_go() {
    let db = dirs::data_dir()
        .expect("data_dir")
        .join("com.tokencounter.gateway")
        .join("app.db");
    let conn = crate::db::open_and_ensure(&db).expect("打開真實資料庫");

    // (模型名, 期望協議)
    let cases = ["claude-haiku-5-5", "deepseek-v4.1-flash"];
    let mut checked = 0;
    for name in cases {
        let row: Option<(i64, i64)> = conn
            .query_row(
                "SELECT m.provider_id, m.id FROM provider_models m
                 JOIN providers p ON p.id = m.provider_id
                 WHERE m.actual_model = ?1 AND p.enabled = 1 LIMIT 1",
                [name],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .ok();
        let Some((pid, mid)) = row else {
            println!("（跳過 {name}：資料庫裡找不到）");
            continue;
        };
        let cfg = match probe_read_cfg(&conn, pid, mid) {
            Ok(c) => c,
            Err(e) => {
                println!("（跳過 {name}：{e}）");
                continue;
            }
        };
        let rep = probe_protocol_http(&cfg);
        println!(
            "{}：protocol={:?} definitive={} requests={}  {}",
            name,
            rep.protocol,
            rep.definitive,
            rep.requests,
            rep.summary
        );
        for a in &rep.attempts {
            println!("    {} -> HTTP {} ({})", a.protocol, a.status, a.outcome);
        }
        if name == "claude-haiku-5-5" {
            // 這一條是穩定的：它確實只吃 Anthropic，而且 chat／responses 是
            // 明確的 400 拒絕（不是暫時故障）
            assert_eq!(rep.protocol.as_deref(), Some("anthropic"), "{name}");
            assert!(rep.definitive, "400 拒絕是可判定的：{rep:?}");
        } else {
            // 這一條**不能**斷言成 anthropic-only。
            //
            // 2026-10-10 真機測試就是在這裡抓到探測器的缺陷：chat／responses
            // 剛好回 502（上游暫時故障），舊版把它當成「協議不支援」，於是
            // 判定 deepseek「只走 Anthropic」—— 而目錄排除是破壞性的，
            // 會把一個好好的模型從 Codex 下架。
            assert!(
                !(rep.protocol.as_deref() == Some("anthropic") && rep.definitive),
                "{name} 不該被判定成「只走 Anthropic」且確定：{rep:?}"
            );
            if rep.protocol.as_deref() == Some("chat") {
                println!("    （chat 正常，符合手動量測）");
            } else {
                println!("    （暫時性失敗，未下結論 —— 這正是要保護的行為）");
            }
        }
        checked += 1;
    }
    assert!(checked > 0, "沒有任何模型被驗證到（來源或金鑰不在？）");
}
