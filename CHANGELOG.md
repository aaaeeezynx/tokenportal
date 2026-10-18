# Changelog

本檔記錄 Token Toll 每個版本的變更。格式參照 [Keep a Changelog](https://keepachangelog.com/zh-TW/1.1.0/)，版本號遵循 [SemVer](https://semver.org/lang/zh-TW/)。

---

## [0.1.3] — 2026-10-19

### 新增

- **一鍵自動偵測連線設定**：來源編輯頁新增「自動偵測」。只填地址與金鑰，即可自動判斷 base_url、逐協議路徑前綴、以及每個模型走哪個協議；已存在的來源可一鍵重新適配。
- **逐協議路徑前綴**（`providers.protocol_prefixes`）：一個來源可同時服務四種協議（例：OpenCode Go 的 `/inference/go` 下掛 `/openai/v1`、`/anthropic/v1`、`/google/v1beta`）。
- **OpenAI ⇄ Gemini 反向翻譯**：Codex／Chat 客戶端可直接使用 Gemini 上游。新增 `ChatToGemini`、`ResponsesToGemini` 兩種轉換（Responses 走兩跳）。
- **逐模型 Gemini 協議**：`provider_models.protocol` 可設為 `gemini`，與同來源的其他模型分開走協議。
- **空回包現場傾印**：上游回 4xx 卻沒有任何內容時，自動把送出的請求體存到 `%APPDATA%\com.tokencounter.gateway\dumps\`（保留最近 20 份），錯誤訊息會附上檔案路徑。

### 修正

- **Gemini 工具呼叫的 `thoughtSignature` 完整往返**：Gemini 要求把模型回傳的簽章在下一輪原樣送回。原本被丟棄，導致模型一旦呼叫工具，之後每一輪都 400。串流端點對這個錯誤回**空 body**，所以只看到「上游返回錯誤（空回包）」。現在簽章夾在 tool-call id 裡往返；取不到時退到官方哨兵 `skip_thought_signature_validator`。
- **Gemini 工具參數 schema 過白名單**：Gemini 的 `functionDeclarations.parameters` 是嚴格解析的 OpenAPI 子集，遇到不認得的關鍵字（`additionalProperties`、`$schema`、`strict` 等）整顆請求 400。現在只保留 Gemini 認得的關鍵字。
- **Gemini 工具結果的 `functionResponse.name` 由 `tool_call_id` 反查**：Chat 的 tool 訊息只帶 id，Gemini 要的是函式名，留空會 400。
- **思考強度探測改依模型協議選機制**：`auto` 來源的所有模型先前都被擋下（舊版以來源級 `api_format` 判斷）。
- **Gemini 探測改試多種路徑形狀**：不再誤報「四種協議都不通」。
- **OpenCode Go 用量卡片消失**：來源 base 改為 `/inference/go` 後偵測不到；用量端點也改為 `/v1/usage`。

### 變更

- **來源編輯頁移除三項設定**：接口格式、逐協議路徑前綴、鑑權方式，改由自動偵測決定。欄位仍保留於資料庫，以支援匯入／匯出與偵測寫入。
- **批次思考強度偵測輸出簡化**：改為 `模型名稱：low / medium / high` 一行式，並以進度條顯示進度（與用量頁同一套設計語言）。
- 按鈕改名為「偵測思考強度」，高度固定為單行。

### 移除

- 死碼清理：`fsutil::backup_text`、`AppError::into_message`、`AUTH_SCHEMES`、`THEME_LABEL`、`Dict`、`DICT_SIZE`，以及 5 條未被引用的 i18n 詞條。

---

## [0.1.2] — 2026-10-11

### 新增

- **相容性矩陣補全**：`anthropic` 渠道可被 OpenAI Chat 與 Codex（Responses）客戶端使用，經自動翻譯（Responses 側兩跳）。Anthropic 的思考區塊刻意不外傳，避免上游回「找不到 reasoning item」。
- **新來源預設 `auto`**：來源格式 `auto` 依逐模型偵測結果換手（`openai-chat` → `openai-responses` → `anthropic`），同一訂閱可逐模型混用協議，不必拆成多個來源。
- **模型協議與思考強度實測**：可逐模型或一鍵實測「走哪個協議、支援哪些思考檔位」，結果落庫。
- **21 項版面修復**：工具列間距、標題間距、對話框關閉鈕位置、統計數值自動縮字、窄視窗對齊等。
- **等寬數字修復**：26 處 `tabular-nums` 真正生效。
- **警示色 token 化**：錯誤／警告／成功統一為 `--mac-red`／`--mac-amber`／`--mac-green`；淺色主題下警示文字對比達 WCAG AA。

---

## [0.1.1] — 2026-10-08

### 修正

1. **換協議撞到 404 被誤學成「協議會通」→ 永久 404**：只有 2xx 才學習路由，非 4xx 才算成功。
2. **Codex 模型目錄模板自癒**：不再依賴外部工具 cc-switch。
3. **修掉外部指令的 pipe 死結**。
4. **上下文視窗改為下拉選單，預設 256K**。

---

## [0.1.0] — 2026-10-04

首個公開發布。
