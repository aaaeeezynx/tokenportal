//! 送出上游與「400 剝離重試」。
//!
//! ⚠️ 本模組的函式**不能**接受 `&rusqlite::Connection`：`Connection`
//! 是 `Send` 但**不是** `Sync`，`&Connection` 因此不是 `Send`，會讓
//! axum 的 `Handler` 靜默不成立（編譯器不會提 Send）。這裡一律開
//! 短命連線或只收 `db_path`。

use super::*;

/// 上游連線資訊，並負責實際發送。
///（把 client／method／url／headers 綁成一個物件，免得 send 變成一長串參數。）
pub(super) struct Upstream<'a> {
    pub(super) client: &'a reqwest::Client,
    pub(super) method: reqwest::Method,
    pub(super) url: String,
    pub(super) headers: Vec<(String, String)>,
}
impl Upstream<'_> {
    /// 送出請求。`pub(super)`：budget 整流器（`retry_budget.rs`）也要用。
    pub(super) async fn send(&self, body: Vec<u8>) -> Result<reqwest::Response, reqwest::Error> {
        let mut b = self.client.request(self.method.clone(), &self.url);
        for (k, v) in self.headers.iter() {
            b = b.header(k, v);
        }
        // 強制 identity：上游若壓縮，我們得先解壓才能解析 SSE。
        b = b.header("accept-encoding", "identity");
        b.body(body).send().await
    }
}
/// 400 重試階段所需的請求脈絡（留痕與拒絕都要用）。
///
/// 共用的 4 個欄位（`ctx`／`started`／`app`／`model_raw`）來自 [`ReqCtx`]；
/// 這裡只補本階段額外需要的轉譯脈絡。
pub(super) struct RetryCtx<'a> {
    pub(super) req: ReqCtx<'a>,
    pub(super) content_type: &'a str,
    pub(super) in_fmt: InFmt,
    pub(super) target_fmt: TargetFmt,
    pub(super) kind: TransKind,
}
impl RetryCtx<'_> {
    /// 建立一筆本請求的追蹤記錄（尚未寫入）。
    /// `retry_count` 由「有沒有真的剝掉東西」推導 —— 有剝才叫重試。
    pub(super) fn trace(&self, status: u16, stripped: Vec<String>, note: &str) -> TraceRecord {
        TraceRecord {
            app: self.req.app.to_string(),
            model_raw: self.req.model_raw.to_string(),
            in_fmt: self.in_fmt.as_str().to_string(),
            target_fmt: self.target_fmt.as_str().to_string(),
            trans_kind: self.kind.as_str().to_string(),
            upstream_status: status,
            latency_ms: self.req.started.elapsed().as_millis() as i64,
            retry_count: if stripped.is_empty() { 0 } else { 1 },
            stripped_fields: stripped,
            content_type: self.content_type.to_string(),
            note: note.to_string(),
            ..Default::default()
        }
    }

    pub(super) fn connect_failed(&self, e: &reqwest::Error) -> Response {
        reject(
            self.req.ctx,
            self.req.started,
            self.req.app,
            self.req.model_raw,
            StatusCode::BAD_GATEWAY,
            format!("上游連接失敗：{e}"),
        )
    }
}
/// 送出上游的結果分類。
///
/// 會分類而不是直接回 `Result`，是因為「協議不支援」需要**呼叫端**換協議或換來源
/// 再試一次（見 `failover.rs`）—— 這一層拿不到別的協議的請求體，只能把訊號往上傳。
pub(super) enum SendOutcome {
    /// 上游有回應（含上游自己的 4xx／5xx），照原樣交給收尾階段。
    Ok(reqwest::Response),
    /// 已備好的拒絕回應（連線失敗、剝離後仍失敗、非協議類 400）。
    Reject { resp: Box<Response>, retryable: bool },
    /// 上游明說「這個模型不支援這個協議」：換協議／換來源再試。
    ProtocolMismatch,
}

/// 上游錯誤訊息是否為「協議不支援」。
///
/// opencode-go（zen 系）回的是
/// `{"type":"error","error":{"type":"ModelProtocolUnsupported","message":"Model does not support this protocol."}}`；
/// 兩種寫法都認，因為同族上游的 `type` 與 `message` 不一定同時出現。
pub(super) fn is_protocol_mismatch(body: &str) -> bool {
    body.contains("ModelProtocolUnsupported") || body.contains("does not support this protocol")
}
/// 送出請求；若上游回 400 且能從錯誤訊息解析出拒收欄位名，記住該欄位、
/// 剝離後重試一次。回傳最終要用的上游響應分類
/// （`Box<Response>`：`Response` 很大，直接當 Err 會讓 Result 過胖，
/// 見 clippy::result_large_err）。
///
/// 學到的欄位持久化於 SQLite 的 `provider_stripped_fields`：原實作只存在
/// 進程記憶體，網關每次停止／啟動即歸零，導致每個渠道的第一個請求都要
/// 重踩一次 400 再重試（見 §3 B2）。正常情況下 `strip_for_upstream` 已先
/// 套用這份記憶，所以不會走到這裡。
///
/// 2026-09-30：先判「協議不支援」再談剝離 —— 協議不對時剝欄位永遠不會成功，
/// 硬剝只是白花一次上游呼叫（trace 一直是「無法解析出拒收欄位名」那條）。
pub(super) async fn send_with_strip_retry(
    rc: &RetryCtx<'_>,
    provider_id: i64,
    up: &Upstream<'_>,
    body: Vec<u8>,
) -> SendOutcome {
    let upstream = match up.send(body.clone()).await {
        Ok(r) => r,
        Err(e) => {
            return SendOutcome::Reject {
                resp: Box::new(rc.connect_failed(&e)),
                retryable: true,
            }
        }
    };
    if upstream.status() != StatusCode::BAD_REQUEST {
        return SendOutcome::Ok(upstream);
    }
    // 仍失敗或無法解析則透出上游原文，不再吞錯。
    let eb: Vec<u8> = upstream.bytes().await.unwrap_or_default().to_vec();
    // 客戶端訊息用截斷版；追蹤表存完整原文（§5.3 第 0 層）
    let upstream_text = upstream_err_text(&eb);
    let upstream_full = String::from_utf8_lossy(&eb).to_string();
    if is_protocol_mismatch(&upstream_full) {
        let rec = rc
            .trace(
                400,
                vec![],
                "上游 400：此模型不支援本協議（換協議／換來源重試中）",
            )
            .with_body(&body)
            .with_upstream_error(&upstream_full);
        trace::log_to(&rc.req.ctx.db_path, &rec);
        return SendOutcome::ProtocolMismatch;
    }
    let mut retried: Option<reqwest::Response> = None;
    let mut applied: Vec<String> = vec![];
    // ⓪ 先處理「這個推理簽章不是我發的」：`store: false` 的請求我們會**保留**
    //    `encrypted_content`（那是上游重播推理的唯一依據），但若這個來源真的
    //    拒收簽章，就得剝掉重試並**記住這個來源要剝**，下次連無狀態請求也剝。
    //    見 `strip_for_upstream` 的長註解與 `strip::is_stateless`。
    if is_encrypted_content_rejected(&upstream_full) {
        if let Some(nb) = strip_encrypted_content(&body) {
            let mem = open_conn(&rc.req.ctx.db_path).ok();
            match mem
                .as_ref()
                .map(|c| trace::remember_stripped(c, provider_id, ENCRYPTED_CONTENT_FIELD))
            {
                Some(Err(e)) => eprintln!("gateway: 記錄拒收簽章失敗: {e}"),
                None => eprintln!("gateway: 記錄拒收簽章失敗：資料庫不可用"),
                Some(Ok(())) => {}
            }
            drop(mem);
            let rec = rc
                .trace(
                    400,
                    vec![ENCRYPTED_CONTENT_FIELD.to_string()],
                    "上游 400：拒收推理簽章，已剝離 encrypted_content 並重試",
                )
                .with_body(&body)
                .with_upstream_error(&upstream_full);
            trace::log_to(&rc.req.ctx.db_path, &rec);
            match up.send(nb).await {
                Ok(r) => return SendOutcome::Ok(r),
                Err(e) => {
                    return SendOutcome::Reject {
                        resp: Box::new(rc.connect_failed(&e)),
                        retryable: true,
                    }
                }
            }
        }
    }
    // ⓪½ Thinking Signature 整流（見 rectifier.rs）：多輪對話的歷史思考塊
    //    帶著過期／被改的簽章回來，上游 400 且每次續聊都失敗。偵測到簽章類
    //    錯誤就對送出去的 body 做最小侵入修復後重試一次。
    //    **刻意不記住**：剝欄位是「這個來源永遠不收」，簽章錯誤是「這一次的
    //    簽章壞了」—— 記住會讓之後所有請求都盲目剝簽章（見 rectifier.rs）。
    if is_signature_error(&upstream_full) {
        if let Some(nb) = rectify(&body) {
            let rec = rc
                .trace(
                    400,
                    vec![],
                    "上游 400：簽章類錯誤，已移除歷史 thinking／簽章並重試",
                )
                .with_body(&body)
                .with_upstream_error(&upstream_full);
            trace::log_to(&rc.req.ctx.db_path, &rec);
            match up.send(nb).await {
                Ok(r) => return SendOutcome::Ok(r),
                Err(e) => {
                    return SendOutcome::Reject {
                        resp: Box::new(rc.connect_failed(&e)),
                        retryable: true,
                    }
                }
            }
        }
    }
    let fields = parse_unknown_fields(&String::from_utf8_lossy(&eb));
    if !fields.is_empty() {
        let mut nb = body.clone();
        // 用短命連線寫記憶，而不是借用呼叫端的連線：
        // `rusqlite::Connection` 是 Send 但**不是** Sync，所以 `&Connection`
        // 跨 await 會讓整個 future 變成 !Send，axum 的 Handler 就不成立。
        // 開新連線也與 `trace::log_to` 的既有做法一致。
        let mem = open_conn(&rc.req.ctx.db_path).ok();
        for field in &fields {
            if let Some(n) = strip_json_field(&nb, field) {
                match mem.as_ref().map(|c| trace::remember_stripped(c, provider_id, field)) {
                    Some(Err(e)) => eprintln!("gateway: 記錄拒收欄位失敗: {e}"),
                    None => eprintln!("gateway: 記錄拒收欄位失敗: 資料庫不可用"),
                    Some(Ok(())) => {}
                }
                nb = n;
                applied.push(field.clone());
            }
        }
        drop(mem);
        if !applied.is_empty() {
            // 剝離事件留痕：這是回答「這個 400 到底剝了什麼」的唯一來源。
            // 記的是**原始** body，不是剝完的 nb（否則看不出剝了什麼）。
            let rec = rc
                .trace(400, applied.clone(), "上游 400 拒收欄位，已剝離並重試")
                .with_body(&body)
                .with_upstream_error(&upstream_full);
            trace::log_to(&rc.req.ctx.db_path, &rec);
            match up.send(nb).await {
                Ok(r) => retried = Some(r),
                Err(e) => {
                    return SendOutcome::Reject {
                        resp: Box::new(rc.connect_failed(&e)),
                        retryable: true,
                    }
                }
            }
        }
    }
    match retried {
        Some(r) if !r.status().is_client_error() => SendOutcome::Ok(r),
        Some(r) => {
            let st = r.status();
            let eb2 = r.bytes().await.unwrap_or_default().to_vec();
            let eb2_full = String::from_utf8_lossy(&eb2).to_string();
            // 剝離後仍失敗 → 這是真正未解決的 400，完整留痕
            // 上游什麼都沒說 → 把我們送出去的原文存檔，否則事後無法重現
            let dumped = dump_if_silent(rc, &body, &eb2);
            let note = match &dumped {
                Some(p) => format!("剝離後重試仍失敗（未解決）；現場已存到 {}", p.display()),
                None => "剝離後重試仍失敗（未解決）".to_string(),
            };
            let rec = rc
                .trace(st.as_u16(), applied.clone(), &note)
                .with_body(&body)
                .with_upstream_error(&eb2_full);
            trace::log_to(&rc.req.ctx.db_path, &rec);
            SendOutcome::Reject {
                resp: Box::new(reject(
                    rc.req.ctx,
                    rc.req.started,
                    rc.req.app,
                    rc.req.model_raw,
                    st,
                    upstream_err_text(&eb2),
                )),
                retryable: is_protocol_mismatch(&eb2_full),
            }
        }
        None => {
            // 在宣告「相容策略失效」之前，先試一個很常見的原因：**思考強度不支援**。
            //
            // 上游對不支援的 `reasoning_effort` 回的是泛用的
            // `400 Invalid request parameters`，訊息裡沒有欄位名，所以上面那條
            // 「剝離未知欄位」永遠學不到東西（2026-10-08 的兩個 MiMo 就是這樣）。
            // 但**我們知道自己送了什麼** —— 拿掉它重試一次，並把「這個模型不收
            // 這個強度」記進 learn 表。零額外請求：真的踩到才會發生。
            if let Some(o) = retry_without_reasoning(rc, provider_id, up, &body, &upstream_full).await
            {
                return o;
            }
            // 無法從錯誤訊息解析出欄位名 → 相容策略失效，必須留痕才能改進
            // 空回包 = 沒有任何線索。把送出的原文存檔，並在訊息裡指出位置。
            let dumped = dump_if_silent(rc, &body, &eb);
            let base = if applied.is_empty() {
                "上游 400 且無法解析出拒收欄位名（相容策略失效）"
            } else {
                "上游 400（剝離未命中任何欄位）"
            };
            let note = match &dumped {
                Some(p) => format!("{base}；現場已存到 {}", p.display()),
                None => base.to_string(),
            };
            let rec = rc
                .trace(400, applied.clone(), &note)
                .with_body(&body)
                .with_upstream_error(&upstream_full);
            trace::log_to(&rc.req.ctx.db_path, &rec);
            SendOutcome::Reject {
                resp: Box::new(reject(
                    rc.req.ctx,
                    rc.req.started,
                    rc.req.app,
                    rc.req.model_raw,
                    StatusCode::BAD_REQUEST,
                    match &dumped {
                        Some(p) => format!("{upstream_text}（送出的請求已存到 {}）", p.display()),
                        None => upstream_text,
                    },
                )),
                retryable: false,
            }
        }
    }
}

/// 上游回 4xx **卻一個字都沒說**時，把送出的請求體存檔。
///
/// 只有「空回包」才存 —— 上游有給訊息時，訊息本身就是線索，不必倒一份
/// 可能很長的對話到磁碟。
fn dump_if_silent(
    rc: &RetryCtx<'_>,
    body: &[u8],
    upstream: &[u8],
) -> Option<std::path::PathBuf> {
    if !upstream.iter().all(|b| b.is_ascii_whitespace()) {
        return None;
    }
    super::dump::dump_upstream_failure(&rc.req.ctx.db_path, body, upstream)
}
