//! 逐模型協議的**儲存與訊息**測試。
//!
//! 從 `protocol_probe.rs` 拆出來（2026-10-11）：加入 Gemini 與正規化的
//! 案例之後那支超過專案 400 行的自律上限。測試內容一字未改。
//!
//! 這裡測的是「值怎麼存、舊資料怎麼救、錯誤訊息怎麼講」；探測器本身的
//! 判定邏輯（哪種回應算支援、暫時性失敗不能當拒絕）在 `protocol_probe`。

use super::*;

/// ★ 迴歸（2026-10-11）：探測器寫的是**短名**，欄位與路由要的是**長名**。
///
/// 原本直接把短名寫進 `provider_models.protocol`，於是 `formats_for` 拿到
/// `"chat"` → `TargetFmt::from_db` 認不得 → `Unknown` → 走**直通**而不是翻譯，
/// Responses 客戶端被送到 `/responses` 而不是 `/chat/completions`。
#[test]
fn probe_stores_long_protocol_names_and_tolerates_old_short_ones() {
    assert_eq!(to_stored_protocol("chat"), "openai-chat");
    assert_eq!(to_stored_protocol("responses"), "openai-responses");
    // 兩邊同名的原樣保留
    assert_eq!(to_stored_protocol("anthropic"), "anthropic");
    assert_eq!(to_stored_protocol("gemini"), "gemini");
    // 已經正規化的值不會被再改一次（冪等）
    assert_eq!(to_stored_protocol("openai-chat"), "openai-chat");

    // 寫入 → 存的是長名
    let dir = tempfile::tempdir().unwrap();
    let conn = crate::db::open_and_ensure(&dir.path().join("n.db")).unwrap();
    let pid = seed_provider(&conn, "norm");
    seed_model(&conn, pid, "m", None);
    let rep = ProtocolProbeReport {
        model: "m".into(),
        protocol: Some("chat".into()),
        definitive: true,
        ..Default::default()
    };
    probe_protocol_write(&conn, pid, &rep).unwrap();
    let stored: String = conn
        .query_row(
            "SELECT protocol FROM provider_models WHERE provider_id=?1 AND actual_model='m'",
            [pid],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(stored, "openai-chat", "欄位裡必須是長名");

    // 舊資料（短名）在**讀取**時也要被容忍 —— 使用者不必做任何事
    conn.execute(
        "UPDATE provider_models SET protocol='responses' WHERE actual_model='m'",
        [],
    )
    .unwrap();
    assert_eq!(
        model_protocol(&conn, pid, "m").as_deref(),
        Some("openai-responses"),
        "舊的短名要能讀成長名"
    );
}

/// ★ 舊資料的自癒：`open_and_ensure` 會把短名就地正規化。
#[test]
fn open_normalizes_legacy_short_protocol_values() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("legacy.db");
    // 注意：資料庫有種子來源，所以 provider id 不是 1 —— 要用實際拿到的。
    let pid;
    {
        let conn = crate::db::open_and_ensure(&path).unwrap();
        pid = seed_provider(&conn, "legacy");
        seed_model(&conn, pid, "a", None);
        seed_model(&conn, pid, "b", None);
        // 模擬上一版留下的壞資料
        conn.execute(
            "UPDATE provider_models SET protocol='chat' WHERE actual_model='a'",
            [],
        )
        .unwrap();
        conn.execute(
            "UPDATE provider_models SET protocol='responses' WHERE actual_model='b'",
            [],
        )
        .unwrap();
        crate::trace::remember_protocol(&conn, pid, "a", PROBED_PROTOCOL_KEY, "chat").unwrap();
    }
    // 重新開啟 → 正規化
    let conn = crate::db::open_and_ensure(&path).unwrap();
    let a: String = conn
        .query_row(
            "SELECT protocol FROM provider_models WHERE actual_model='a'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let b: String = conn
        .query_row(
            "SELECT protocol FROM provider_models WHERE actual_model='b'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(a, "openai-chat");
    assert_eq!(b, "openai-responses");
    // 探測記憶那份也要一起修
    assert_eq!(
        probed_protocol(&conn, pid, "a").as_deref(),
        Some("openai-chat"),
        "provider_model_protocol 也要正規化"
    );
}

/// 哨兵鍵不會干擾換手用的逐宣告記憶。
#[test]
fn probe_key_does_not_shadow_declared_protocol_memory() {
    let dir = tempfile::tempdir().unwrap();
    let conn = crate::db::open_and_ensure(&dir.path().join("k.db")).unwrap();
    let pid = seed_provider(&conn, "k");
    crate::trace::remember_protocol(&conn, pid, "m", PROBED_PROTOCOL_KEY, "anthropic").unwrap();
    crate::trace::remember_protocol(&conn, pid, "m", "mixed", "openai-chat").unwrap();
    // 換手讀的是宣告鍵，不受哨兵影響
    assert_eq!(
        crate::trace::load_protocol(&conn, pid, "m", "mixed").as_deref(),
        Some("openai-chat")
    );
    assert_eq!(probed_protocol(&conn, pid, "m").as_deref(), Some("anthropic"));
}

/// 候選全滅時的訊息要講出真正的原因，而不是「請換模型或換來源」。
///
/// 2026-10-11 更新：反向翻譯器實作後，Anthropic-only 的模型**不再**是
/// 「Codex 到不了」，所以訊息改成指向最可能的原因（鑑權）。
#[test]
fn mismatch_message_explains_anthropic_only_models() {
    let dir = tempfile::tempdir().unwrap();
    let conn = crate::db::open_and_ensure(&dir.path().join("m.db")).unwrap();
    let pid = seed_provider(&conn, "msg");
    crate::trace::remember_protocol(&conn, pid, "claude-haiku-5-5", PROBED_PROTOCOL_KEY, "anthropic")
        .unwrap();

    let msg = protocol_mismatch_message(Some(&conn), pid, "claude-haiku-5-5", 2);
    assert!(msg.contains("Anthropic"), "{msg}");
    assert!(msg.contains("翻譯"), "要說明網關已經翻譯過了：{msg}");
    assert!(msg.contains("x-api-key"), "要指向最可能的原因（鑑權）：{msg}");
    // 這句話已經不成立了 —— 翻譯器做好之後 Codex 到得了
    assert!(!msg.contains("Codex 到不了"), "舊說法已過時：{msg}");
    assert!(!msg.contains("請換模型或換來源"), "別再給那句沒用的建議：{msg}");

    // 沒測過 → 引導去按偵測，而不是空泛地叫人換模型
    let msg2 = protocol_mismatch_message(Some(&conn), pid, "unknown-model", 2);
    assert!(msg2.contains("偵測"), "{msg2}");
    // 資料庫不可用也要給得出話
    let msg3 = protocol_mismatch_message(None, pid, "x", 1);
    assert!(!msg3.is_empty());
}
