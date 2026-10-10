//! 真機驗證：協議探測對真實 opencode-go 的判斷。
//!
//! 2026-10-10 手動量測的結論：`claude-haiku-5-5` 只走 Anthropic Messages
//! （`/messages` + `x-api-key`），`/chat/completions` 回
//! `ModelProtocolUnsupported`；而 `deepseek-v4.1-flash` 走 chat。
//! 探測器必須自己得出同一組答案。
//!
//! `cargo test --offline --lib -- --ignored live_probe_protocol --nocapture`

use super::*;

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
