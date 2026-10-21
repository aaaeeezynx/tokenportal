# 接手 Prompt · Token Portal（給新裝置上的 AI agent）

把本文件整份貼給新裝置上的 agent 即可。內容自足，不需要其他上下文。

---

## 你的任務

在一台**全新的 Windows 裝置**上接手這個專案：讓它跑起來、能除錯、並在需要時發佈新的修正版（patch release）。

請照下面的順序做，遇到與文件不符的地方**停下來回報**，不要自行猜測後硬幹。

---

## 0. 先確認你面對的是什麼

| 項目 | 值 |
|---|---|
| 產品名稱 | **Token Portal**（本機 AI 用量收費站） |
| 用途 | 本機反向代理：把多個上游 AI 供應商聚成一個 OpenAI／Anthropic／Gemini 相容端點，逐工具簽發本地 Key，計量與計價每一次請求 |
| 技術 | Tauri 2 ＋ React 19 ＋ Rust；**僅 Windows x64** |
| Repo | `https://github.com/aaaeeezynx/tokenportal.git` |
| 分支 | `main`（唯一分支） |
| 目前版本 | `0.1.3`（三處一致），但**已有 7 個未發佈的功能 commit** |
| 已發佈 | v0.1.0 / v0.1.1 / v0.1.2 / v0.1.3 |
| 目前 HEAD | `255c0f7`（與 `origin/main` 同步） |

**下一個版本是 `0.1.4`。** 這正是「新修正檔」的目標。

`v0.1.3` 之後的 7 個未發佈 commit：

```
255c0f7  Rectifier —— Thinking Signature 整流器
3478c94  Anthropic → Gemini 兩跳，四協議矩陣全通
90a855f  Claude Desktop 支援
32d1f5d  CLI 工具管理（一鍵安裝／升級／重複安裝診斷）
d6c89db  內建小工具（跳過初次確認／隱藏 AI 署名／擴充整合）
24c4f61  從 CC Switch 同步餘額／套餐額度樣板（6 → 10）
3429637  從 CC Switch v4.0.4 同步來源預設集（38 → 195）
```

### 命名沿革（讀程式碼時會遇到三個名字，不要混淆）

```
Token Gateway  →  Token Toll  →  Token Portal
（2026-10-04 前）  （~10-19）      （現在）
```

舊名**刻意保留**在相容識別碼裡（見 §6），那不是漏改。

---

## 1. 環境建置

### 必要工具

| 工具 | 本專案驗證過的版本 | 備註 |
|---|---|---|
| Windows | 10 1809+ / 11（x64） | 只支援 Windows |
| Node.js | v24.20.0 | |
| pnpm | 10.34.5 | **必須用 pnpm**，不要用 npm／yarn（有 pnpm-lock.yaml） |
| Rust | 1.98.0（cargo ＋ rustc） | MSVC toolchain |
| MSVC Build Tools | — | 需「C++ 建置工具」工作負載，否則連結會失敗 |
| WebView2 Runtime | — | Win11 通常內建 |
| git | 2.54.0 | |

Tauri 首次建置會**自動下載** NSIS 與 WiX 工具組，不必手動安裝。

### 取得專案並跑起來

```powershell
git clone https://github.com/aaaeeezynx/tokenportal.git
cd tokenportal
git log --oneline -12          # 先看清楚目前進度

cd app
pnpm install                   # 首次會裝約 440 MB
```

### 驗證建置環境（四道閘門，基線數字必須完全一致）

```powershell
# 1) Rust 測試
cd app\src-tauri
cargo test --offline --lib
#    預期：463 passed / 0 failed / 17 ignored

# 2) Rust lint
cargo clippy --offline --all-targets -- -D warnings
#    預期：0 warning

# 3) TypeScript 型別檢查
cd ..
npx tsc --noEmit
#    預期：exit 0

# 4) 完整建置與打包
pnpm tauri build
#    預期產出（版本號依當時而定，目前是 0.1.3）：
#      app\src-tauri\target\release\bundle\nsis\Token Portal_0.1.3_x64-setup.exe
#      app\src-tauri\target\release\bundle\msi\Token Portal_0.1.3_x64_en-US.msi
#    建置約 5–15 分鐘
```

**數字對不上就先停下來查，不要繼續。** 這是判斷「環境有沒有問題」最快的方法。

### 開發時執行（不打包）

```powershell
cd app
pnpm tauri dev          # 熱重載；會開一個開發視窗
```

---

## 2. 專案結構

```
.
├── app/                          # 應用程式本體（Tauri 2）
│   ├── src/                      #   React ＋ TypeScript 前端
│   │   ├── components/           #     各頁面與元件
│   │   ├── lib/                  #     API 封裝、型別、i18n、主題
│   │   └── styles/
│   ├── src-tauri/
│   │   ├── src/
│   │   │   ├── proxy/            #     HTTP 代理、協議矩陣、故障轉移、整流器
│   │   │   │   ├── matrix.rs     #       ★ 協議轉換矩陣（改協議前必讀）
│   │   │   │   ├── rectifier.rs  #       簽章整流（見 §3）
│   │   │   │   ├── strip.rs      #       欄位剝除與相容策略
│   │   │   │   ├── retry.rs / retry_budget.rs
│   │   │   │   ├── failover.rs   #       候選來源排序
│   │   │   │   ├── dump.rs       #       空回包現場傾印（見 §4）
│   │   │   │   └── pipeline.rs / forward*.rs / finish*.rs / stream.rs
│   │   │   ├── translate/        #     四協議互轉（都是純函式，好測）
│   │   │   ├── tools/            #     工具接管與還原（Codex／Claude／OpenCode／Claude Desktop）
│   │   │   ├── models/           #     模型目錄、協議與思考強度探測
│   │   │   ├── quota/            #     訂閱額度查詢
│   │   │   ├── providers_io/     #     來源匯出／匯入
│   │   │   ├── claude_desktop/   #     Claude Desktop 整合
│   │   │   ├── usage/ usage_query/
│   │   │   ├── db/               #     SQLite schema 與遷移
│   │   │   ├── trace/            #     proxy_trace 寫入與裁剪
│   │   │   ├── providers.rs / presets.rs / presets_cc.rs / utilities.rs
│   │   │   └── bin/gw.rs         #     純代理執行檔（不開 GUI）
│   │   ├── Cargo.toml
│   │   └── tauri.conf.json
│   ├── docs/screenshots/         #   README 用的截圖
│   └── package.json
├── docs/                         # 開發紀錄（見 §4）
│   ├── TESTING.md                #   ★ 最重要：每個功能的設計取捨與實機驗證（約 6000 行）
│   ├── MANUAL-TESTS.md
│   ├── REFACTORING-PLAN.md
│   ├── SIMPLIFICATION-PLAN.md
│   ├── AGENT-HANDOFF.md          #   本文件
│   └── evidence/                 #   幾次事故的排錯證據
├── scripts/                      # 開發期間的診斷小工具（非執行期程式碼）
├── CHANGELOG.md
├── LICENSE
└── README.md
```

> **`docs/TESTING.md` 是這個專案最有價值的文件。** 它記錄每一輪改動的
> 「使用者症狀 → 真因 → 修法 → 實機驗證」。動手改任何功能前先在裡面搜尋關鍵字，
> 通常會找到當初為什麼這樣寫。
>
> **但它的開頭狀態數字是舊的**（例如寫「228 passed」），因為那是當時的快照。
> 要判斷現況**一律自己跑 §1 的四道閘門**，不要相信文件裡的數字。

---

## 3. 架構重點（改程式前必讀）

### 三個進入點

| 執行檔 | 用途 |
|---|---|
| `app/src-tauri/src/main.rs` → `tokenportal.exe` | GUI 應用程式（含內嵌網關） |
| `app/src-tauri/src/bin/gw.rs` → `gw.exe` | **純代理**，無 GUI，供無介面環境使用 |
| 兩者共用 `tokenportal_lib` | |

### 網關

- 綁定 **`127.0.0.1:15722`**（`src/proxy.rs` 與 `bin/gw.rs`；埠號可用命令列參數覆蓋）
- `axum` 的 `Router::new().fallback(proxy_handler)` —— 所有路徑進同一個處理器
- 連線逾時 10 秒、請求逾時 600 秒、`proxy_trace` 保留 5000 筆

### 協議轉換矩陣（`src/proxy/matrix.rs`）

支援 **OpenAI Chat／OpenAI Responses／Anthropic Messages／Gemini** 四種格式互轉。
判斷入口是 `resolve_trans_kind(InFmt, TargetFmt)`，回傳 `TransKind`：

| `TransKind` | 方向 | 備註 |
|---|---|---|
| `None` | 同格式直通 | |
| `AnthropicToChat` | Anthropic 入 → Chat 上游 | |
| `ResponsesToChat` | Responses 入 → Chat 上游 | |
| `ChatToAnthropic` | Chat 入 → Anthropic 上游 | |
| `ResponsesToAnthropic` | Responses 入 → Anthropic 上游 | 請求兩跳 |
| `ChatToGemini` | Chat 入 → Gemini 上游 | |
| `ResponsesToGemini` | Responses 入 → Gemini 上游 | 請求兩跳 |
| `AnthropicToGemini` | Anthropic 入 → Gemini 上游 | 請求與回應各兩跳 |

> **現況：三種客戶端 × 四種上游全部可達。** 剩下的「Gemini 入站」是**刻意鎖死**的
> —— 沒有 Gemini 原生客戶端在用，所以不支援。

**Responses 相關轉換走兩跳**：Responses → Chat → 目標格式；回應反向亦然。
新增協議組合時，通常是「既有零件再串一層」，先確認零件是否已存在。

### 逐模型協議（重要設計）

- 一個來源可以**逐模型**指定協議：`provider_models.protocol` 為 `''`（自動）或四種協議之一
- 來源層另有**逐協議路徑前綴** `providers.protocol_prefixes`（JSON），
  讓單一 base_url 服務全部四種協議（例：OpenCode Go 的 `/inference/go`）
- 最終 URL ＝ `{base}{prefix}{suffix}`

### 簽章的處理（兩層，別搞混）

這是本專案最容易誤解的地方，兩個機制**互補**、不是重複：

| 層 | 位置 | 做什麼 | 時機 |
|---|---|---|---|
| **往返** | `translate/gemini_resp.rs` → tool-call `id` 夾帶簽章；`translate/gemini_req.rs` 帶回 | 把上游回的 `thoughtSignature` 原樣送回，**讓上游願意收** | 主動、每一輪 |
| **整流** | `proxy/rectifier.rs` | 上游抱怨簽章無效時，**剝掉** `thinking` / `signature` / `thoughtSignature` 再重試 | 被動、踩到才做 |

整流器（`docs/TESTING.md` §0.9.49）比對 7 種錯誤訊息，並**刻意不記住**
「這個來源要剝簽章」——因為簽章錯誤是「**這一次**的簽章壞了」，
下一輪的新簽章可能是好的；記住會讓之後所有請求盲目剝簽章，丟掉有用的狀態。
（相對地，`provider_stripped_fields` 記的是「這個來源**永遠**不收這個欄位」。）

### 資料庫

| 項目 | 值 |
|---|---|
| 位置 | `%APPDATA%\com.tokencounter.gateway\app.db` |
| 目前 schema | **v18** |
| 遷移 | `app/src-tauri/src/db/`，加欄位時**必須**遞增 `SCHEMA_VERSION` 並寫遷移 |

### 除錯用的兩張表

| 表 | 用途 |
|---|---|
| `request_logs` | 每次請求的來源 App、模型、狀態、token、耗時 |
| `proxy_trace` | 轉換種類、上游狀態、剝離欄位、**上游錯誤原文**、備註 |

---

## 4. 除錯方法論（這套流程在本專案屢次奏效，請照用）

### 當使用者回報「上游錯誤」時

**不要先猜。** 這個專案吃過兩次「猜錯方向」的虧（見下方案例）。照下面順序：

**第 1 步：查 `proxy_trace`，看真正的上游狀態**

用 Python 讀（**不要用 PowerShell 讀 SQLite，中文會亂碼**）：

```python
import os, sqlite3
db = os.path.join(os.environ["APPDATA"], "com.tokencounter.gateway", "app.db")
c = sqlite3.connect(f"file:{db}?mode=ro", uri=True); c.row_factory = sqlite3.Row
for r in c.execute("SELECT * FROM proxy_trace ORDER BY id DESC LIMIT 10"):
    d = dict(r)
    for k in ("id","trace_level","app","model_raw","in_fmt","target_fmt",
              "trans_kind","upstream_status","content_length","upstream_error","note"):
        v = d.get(k)
        if v not in (None, "", "null", "[]"):
            print(f"  {k} = {str(v)[:200]}")
    print("---")
```

**第 2 步：若 `upstream_error` 是空的 —— 查傾印檔**

有些上游（尤其 OpenCode Go 的 Gemini 路由）拒收時回 **400 且 body 是空的**。
網關遇到這種「空回包」會把**送出去的請求體**存檔：

```
%APPDATA%\com.tokencounter.gateway\dumps\upstream-400-<時間戳>-<長度>.json
```

裡面有 `request_body`（真正打到上游的原文）。**有這份原文就能直接對上游二分定位。**

> 這個傾印機制是為了對付「上游不給任何線索」而加的。若你遇到無法解釋的錯誤，
> 先確認 `dumps/` 裡有沒有現場；沒有的話，考慮擴大傾印的觸發條件。

**第 3 步：拿真實原文對真實上游二分**

關鍵細節（少一個就會得到誤導性的結果）：

- 要用 **provider 自己的 api_key**（`providers.api_key`），不是本地 `sk-local-…` key
  （用錯會得到 `401 Invalid credential`，讓你以為是別的問題）
- `opencode.ai` 需要標頭 `x-opencode-session`（與 `session-id`），否則回 `MissingSessionID`
- User-Agent 不能是 `Python-urllib`，Cloudflare 會回 `403 error code: 1010`
  （用 `curl/8.4.0` 可過）
- 逐項移除欄位重送，找出「移除哪一項就成功」

**第 4 步：修好後做三件事**

1. 加**迴歸測試**（釘住這次的真因，避免日後被順手改掉）
2. 跑完整閘門
3. **對真實上游做端到端驗證** —— 單元測試用假上游，抓不到真機的嚴格驗證

### 案例：為什麼不能猜（詳見 commit `83a2999` 與 `CHANGELOG.md` 的 0.1.3 段落）

> 使用者回報 Codex 跑 Gemini 模型「**回一句話後**出現上游返回錯誤（空回包）」。
>
> 真因：Gemini 要求把模型回傳的 `thoughtSignature` 在下一輪原樣送回，翻譯器把它丟了
> → **模型一旦呼叫工具，之後每一輪都 400**。而**串流端點對同一個錯誤回空 body**，
> 所以只看到「空回包」。
>
> 前兩輪都在猜（修了 `additionalProperties`、`functionResponse.name` ——
> 那些是**真的 bug，但不是這個症狀**），直到加了傾印、拿到真實原文、
> 對上游二分才定位。
>
> **教訓：看不到的錯誤就先做量測，不要猜。**

---

## 5. 發佈流程（patch release）

### 步驟

**1. 確認狀態乾淨**

```powershell
git status --short          # 必須是空的
git fetch origin
git rev-list --left-right --count HEAD...origin/main   # 必須是 0  0
```

**2. 決定版本號並在三個地方同步更新（必須一致）**

| 檔案 | 欄位 |
|---|---|
| `app/package.json` | `"version"` |
| `app/src-tauri/tauri.conf.json` | `"version"` |
| `app/src-tauri/Cargo.toml` | `[package] version` |

**3. 更新文件**

- `CHANGELOG.md`：新增該版本段落（照既有格式：新增／修正／變更／移除）
- `README.md`：更新「最新版本」區塊的版本號、檔名、大小與版本一致性註記

**4. 跑完整閘門**（見 §1，四道全過）

**5. 建置並記錄產物資訊**

```powershell
cd app
pnpm tauri build
Get-ChildItem src-tauri\target\release\bundle\nsis\*0.1.4*,
              src-tauri\target\release\bundle\msi\*0.1.4* |
  ForEach-Object { "{0}  {1} MB  {2}" -f $_.Name,
      [math]::Round($_.Length/1MB,1),
      (Get-FileHash $_.FullName -Algorithm SHA256).Hash }
```

**6. 提交、打標籤、推送**

```powershell
cd ..
git add -A
git commit -F <訊息檔>          # 用檔案，不要在命令列寫多行中文
git tag -a v0.1.4 -m "Token Portal v0.1.4"
git push origin main
git push origin v0.1.4
```

**7. 發佈 GitHub Release（`gh` CLI 可能未安裝，用 REST API）**

Release 附件命名慣例（**點號**，與 repo 內檔案的空格不同）：

```
Token.Portal_0.1.4_x64-setup.exe      ← NSIS，推薦
Token.Portal_0.1.4_x64_en-US.msi      ← MSI
```

**先決條件：這台裝置必須先完成 GitHub 認證。** 新裝置預設沒有。

憑證存在作業系統的憑證管理器，而 `credential.helper` 是**機器專屬路徑**（在原作者
的機器上指向她自己的 Git 安裝，新裝置不會有）。所以要先設定身分與認證：

```powershell
git config --global user.name  "aaaeeezynx"
git config --global user.email "aaaeeezynx@users.noreply.github.com"

# 認證二選一：
#   a) 安裝 Git Credential Manager，首次 push 時以瀏覽器登入
#   b) 建 Personal Access Token（需 repo 權限），當作密碼使用
```

驗證認證可用（應正常列出 ref，而不是 404 或要求輸入密碼）：

```powershell
git ls-remote origin HEAD
```

確認可用之後，才取用憑證（**不要把 token 印出來或寫進檔案**）：

```powershell
$cred = "protocol=https`nhost=github.com`n`n" | git credential fill
$pw = ($cred | Where-Object { $_ -like 'password=*' }) -replace '^password=', ''
```

API 重點：

- 建 Release：`POST https://api.github.com/repos/aaaeeezynx/tokenportal/releases`
- 上傳附件：`POST https://uploads.github.com/repos/.../releases/{id}/assets?name=<檔名>`
- 標頭需 `Authorization: Bearer <token>`、`Accept: application/vnd.github+json`、
  `X-GitHub-Api-Version: 2022-11-28`
- **建 Release 前先 `GET .../releases/tags/v0.1.4` 確認不存在**，避免重複

**8. 事後驗證（不要跳過）**

```powershell
# 附件的 digest 必須與本機 SHA256 一致
GET https://api.github.com/repos/aaaeeezynx/tokenportal/releases/tags/v0.1.4
#  → 每個 asset 的 digest 欄位比對本機 Get-FileHash

# 下載連結必須可用
Invoke-WebRequest -Method Head -Uri "https://github.com/aaaeeezynx/tokenportal/releases/download/v0.1.4/<檔名>"
```

> **注意**：更換 Release 附件會讓舊檔名的下載連結 404（GitHub 不提供附件改名重導向）。
> 若只是重新打包同一版，考慮保留原檔名。

**9. 本機安裝驗證**

```powershell
# 先關閉執行中的程式，再解安裝舊版、裝新版
Get-Process tokenportal -ErrorAction SilentlyContinue | Stop-Process -Force
& "$env:LOCALAPPDATA\Token Portal\uninstall.exe" /S     # 舊版解除安裝
Start-Process "<新安裝檔>" -ArgumentList /S -Wait
Start-Process "$env:LOCALAPPDATA\Token Portal\tokenportal.exe"
Get-NetTCPConnection -LocalPort 15722 -State Listen     # 確認網關起來
```

---

## 6. 硬性約束（**改了會弄壞使用者的既有安裝**）

### 絕對不可更動的值

| 項目 | 值 | 為什麼 |
|---|---|---|
| Tauri identifier | `com.tokencounter.gateway` | 決定 `%APPDATA%\com.tokencounter.gateway\`。改了＝使用者的資料庫、金鑰、歷史全部找不到 |
| 渠道 ID | `tokentoll`（`GATEWAY_PROVIDER_ID`） | 已寫進使用者的 `~/.codex/config.toml`、`opencode.json`；舊會話歷史按段名引用供應商 |
| 環境變數 | `TOKEN_TOLL_KEY`（`GATEWAY_ENV_KEY`） | 使用者的系統環境變數已寫入此名 |
| 舊渠道 ID | `tokengateway`（`GATEWAY_PROVIDER_ID_LEGACY`） | 更早的相容值 |
| 匯出格式 | `tokenportal/providers` ＋ 兩代舊值 | `EXPORT_KIND`／`EXPORT_KIND_PREV`／`EXPORT_KIND_LEGACY`；**匯入端必須新舊都認** |

位置：`app/src-tauri/src/tools/consts.rs` 與 `app/src-tauri/src/providers_io.rs`

> 這些是**相容介面**，不是品牌名稱。品牌已改成 Token Portal，
> 但這些值刻意保留舊名——改了會讓既有使用者的接管設定與匯出檔失效。

### 品牌名稱目前的位置

若將來還要改名，這些是「使用者看得到」的地方：

- `app/src-tauri/tauri.conf.json`：`productName`、`app.windows[].title`
- `app/src-tauri/src/window.rs`、`app/src-tauri/src/tray.rs`
- `app/src/App.tsx`、`app/src/lib/i18nDict.ts`
- `app/package.json` 的 `name`、`app/src-tauri/Cargo.toml` 的 `name`／`default-run`／`[lib] name`
- `app/src-tauri/src/tools/codex/wire.rs`、`app/src-tauri/src/tools/apply.rs`（寫進工具設定的顯示名）
- 對外 HTTP 識別（User-Agent／session 標頭）
- `README.md`、`CHANGELOG.md`

### 其他規範

- **單檔 400 行上限**（含註解）。超過就拆分模組。
  ⚠️ **目前有 10 個檔案超限**（截至 2026-10-21），最嚴重是
  `src/proxy.rs` 479 行、`src/models/tests/live_protocol.rs` 439 行、
  `src/models/codex_catalog.rs` 434 行。這是已知的技術債，
  改到那些檔案時**順手拆一點**，但不要在無關的改動裡大肆重構。
- 新增功能**必須**附單元測試；修 bug **必須**附迴歸測試
- 註解用繁體中文，說明「為什麼」而非「做什麼」
- Commit 訊息：多段式中文（症狀／真因／修法 ＋ 驗證數字），寫成 UTF-8 檔案後 `git commit -F`
- 版本號三處必須一致（`package.json`／`tauri.conf.json`／`Cargo.toml`）
- 資料庫加欄位要遞增 `SCHEMA_VERSION` 並寫遷移

---

## 7. 環境陷阱（本專案實際踩過的，會浪費你很多時間）

### PowerShell 5.1

| 陷阱 | 症狀 | 對策 |
|---|---|---|
| **BOM-less `.ps1` 被當 ANSI 解讀** | 腳本含中文時出現 `The string is missing the terminator` 等解析錯誤 | 腳本**只寫 ASCII**，中文放 UTF-8 檔案再讀；或用 Python 腳本 |
| **`Measure-Object -Line` 行數少算** | 用 `Get-Content \| Measure-Object -Line` 數行數會**嚴重低估**（曾把 479 行的檔案數成 402 行），導致誤判「符合 400 行上限」 | 用 Python 數 `\n` 的數量，或用 `read` 工具（它會回報 total lines） |
| `Get-Content` 讀 UTF-8 顯示亂碼 | 看到「鈹溾攢」之類 | 只是**顯示**問題，檔案沒壞。用 `read` 工具或 Python 確認 |
| `Select-Object -First N` 截斷管線 | 上游程序收到非零 exit code（看似失敗其實成功） | 別用它判斷成功與否，改看實際輸出 |
| 多行 Python 內嵌在命令列 | 引號解析錯誤 | 一律寫成 `.py` 檔再 `py -3 file.py` |
| `-Path 'src\**\*.rs'` 遞迴不完整 | 漏掉檔案 | 用 `grep`／`glob` 工具，不要用 PowerShell 的 `**` |
| 讀 SQLite | 中文亂碼、無法解析 | 用 Python `sqlite3` |

### pnpm

- `node_modules` 內是**絕對路徑的 Junction**。**整個專案目錄搬移後會全部懸空**，
  症狀是 `tsc` 找不到 `react`。修法：`pnpm install --offline`
  （需 `$env:CI='true'` 讓它能清空重建）

### cargo

- 搬移專案目錄後，Tauri 的 **build script 執行檔被快取且內嵌舊絕對路徑**，
  因 mtime 未變而不重編，症狀是
  `failed to read plugin permissions: ... 系統找不到指定的路徑`。
  修法：刪掉 `app/src-tauri/target/{debug,release}/build` 強制重跑

### 網路

- `opencode.ai` 走 Cloudflare：`Python-urllib` 的 User-Agent 會吃 403 `error code: 1010`
- `opencode.ai` 需要 `x-opencode-session` 標頭，否則 `MissingSessionID`
- 直接打上游時**別用本地 `sk-local-…` key**（會得到 401 Invalid credential）

---

## 8. 建議的第一步行動

1. `git clone` → `pnpm install` → 跑四道閘門，確認是 **463 / 0 / 17**、clippy 0、tsc 0
2. 讀 `docs/TESTING.md` 的目錄結構（`grep '^### ' docs/TESTING.md`）建立全局印象；
   特別看 §0.9.49（Rectifier）與 0.1.3 相關段落
3. 讀 `CHANGELOG.md` 了解 v0.1.3 之後那 7 個功能 commit 做了什麼
4. 若使用者已回報具體錯誤 → 直接跳 §4 的除錯流程
5. 若只是要發新修正檔 → 跳 §5 的發佈流程，版本 `0.1.4`

**遇到不確定的地方就問，不要自行推測後大改。**
這個專案有很多「看起來多餘、其實是相容性考量」的程式碼
（例如那三個舊匯出格式值、兩代渠道 ID、刻意不記住的簽章剝除）。
`docs/TESTING.md` 通常會解釋原因。
