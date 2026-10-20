//! Reasoning-effort（思考強度）整流器。
//!
//! 從 retry.rs 抽出來的（該檔加了簽章整流後超過 400 行上限）。
//! 對標 CC Switch 的 thinking_budget_rectifier：上游對「這個模型不收這個
//! 強度」的 400，記下來並拿掉 reasoning_effort 重試。
//!
//! 與 signature 整流（rectifier.rs）的差別：強度是**逐模型**的屬性，
//! 所以這裡會寫進 models 的記憶（learned）—— 之後這個模型就不送了。

use super::*;

/// 從**送出**（已翻譯／已剝離）的 body 取出 `(model, reasoning_effort)`。
///
/// 兩個都取 `model` 欄位與 `reasoning_effort`：前者是記憶的鍵（實測記憶也是
/// 用上游實際模型名當鍵），後者是要拿掉並記下來的值。
fn sent_reasoning(body: &[u8]) -> Option<(String, String)> {
    let v: serde_json::Value = serde_json::from_slice(body).ok()?;
    let level = v.get("reasoning_effort")?.as_str()?.trim().to_lowercase();
    let model = v.get("model")?.as_str()?.trim().to_string();
    if model.is_empty() || level.is_empty() {
        return None;
    }
    Some((model, level))
}

/// 400 且我們送了 `reasoning_effort` → 記下「這個模型不收這個強度」並拿掉重試。
///
/// 回傳 `Some(outcome)` 表示已經重試過（不論成敗）；`None` 表示這條路不適用，
/// 呼叫端照原本的流程回報。
pub(super) async fn retry_without_reasoning(
    rc: &RetryCtx<'_>,
    provider_id: i64,
    up: &Upstream<'_>,
    body: &[u8],
    upstream_full: &str,
) -> Option<SendOutcome> {
    let (model, level) = sent_reasoning(body)?;
    let mut v: serde_json::Value = serde_json::from_slice(body).ok()?;
    v.as_object_mut()?.remove("reasoning_effort");
    let nb = serde_json::to_vec(&v).ok()?;

    // 短命連線寫記憶（理由同上面：`&Connection` 跨 await 會讓 future 變 !Send）
    match open_conn(&rc.req.ctx.db_path) {
        Ok(conn) => {
            if let Err(e) = crate::models::record(
                &conn,
                provider_id,
                &model,
                &level,
                false,
                "learned",
                crate::fsutil::now_ms(),
            ) {
                eprintln!("gateway: 記錄思考強度失敗: {e}");
            }
        }
        Err(e) => eprintln!("gateway: 記錄思考強度失敗（資料庫不可用）: {e}"),
    }

    let rec = rc
        .trace(
            400,
            vec!["reasoning_effort".to_string()],
            "上游 400：疑似思考強度不支援，已移除 reasoning_effort 重試",
        )
        .with_body(body)
        .with_upstream_error(upstream_full);
    trace::log_to(&rc.req.ctx.db_path, &rec);

    match up.send(nb).await {
        Ok(r) if !r.status().is_client_error() => Some(SendOutcome::Ok(r)),
        Ok(r) => {
            let st = r.status();
            let eb = r.bytes().await.unwrap_or_default().to_vec();
            Some(SendOutcome::Reject {
                resp: Box::new(reject(
                    rc.req.ctx,
                    rc.req.started,
                    rc.req.app,
                    rc.req.model_raw,
                    st,
                    upstream_err_text(&eb),
                )),
                retryable: false,
            })
        }
        Err(e) => Some(SendOutcome::Reject {
            resp: Box::new(rc.connect_failed(&e)),
            retryable: true,
        }),
    }
}
