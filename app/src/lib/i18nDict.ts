/**
 * 翻譯字典：`key → [繁中, English]`。
 *
 * 繁中是 source language（`t()` 找不到 key 時也回繁中原文），
 * 所以「繁中欄」寫的就是畫面上原本的字串 —— 不會出現兩邊對不起來的情況。
 *
 * 命名：`區域.項目`。新增翻譯時照這個規則排，
 * 覆蓋率可用 `.workbuddy/tmp/i18n_cover.py` 量（docs/TESTING.md §0.9.32）。
 */
export const DICT: Record<string, [string, string]> = {
  // ── 導覽 ──
  "nav.usage": ["用量", "Usage"],
  "nav.providers": ["上游來源", "Providers"],
  "nav.keys": ["本地 Key", "Local Keys"],
  "nav.diagnostics": ["診斷", "Diagnostics"],
  "nav.calc": ["試算", "Calculator"],
  "nav.settings": ["設定", "Settings"],

  // ── 頂列 ──
  "top.stop": ["停止", "Stop"],
  "top.start": ["啟動", "Start"],

  // ── 設定：網關 ──
  "settings.gateway.title": ["網關", "Gateway"],
  "settings.gateway.caption": [
    "本機代理監聽（僅 127.0.0.1）",
    "Local proxy (127.0.0.1 only)",
  ],
  "settings.gateway.port": ["監聽連接埠", "Listen port"],
  "settings.gateway.check": ["檢測", "Check"],
  "settings.gateway.autostart": ["自動啟動", "Start with app"],
  "settings.gateway.autostart.hint": [
    "應用程式啟動時自動啟動網關",
    "Start the gateway when the app launches",
  ],
  "settings.gateway.save": ["儲存", "Save"],
  "settings.gateway.hint": [
    "15721 常被 cc-switch 佔用，預設 15722。網關執行時不可改連接埠（需先停止）；改完後已切換工具需重新執行一次切換。",
    "Port 15721 is often taken by cc-switch; we default to 15722. The port can only change while the gateway is stopped; re-run a tool switch afterwards.",
  ],
  "settings.gateway.ok": ["連接埠 {port} 可用", "Port {port} is available"],
  "settings.gateway.saved": [
    "已儲存：連接埠 {port}，開機自啟{auto}",
    "Saved: port {port}, autostart {auto}",
  ],
  "settings.gateway.on": ["開", "on"],
  "settings.gateway.off": ["關", "off"],

  // ── 設定：外觀 ──
  "settings.appearance.title": ["外觀", "Appearance"],
  "settings.appearance.caption": [
    "強調色與主題即時套用全站",
    "Accent and theme apply instantly",
  ],
  "settings.appearance.accent": ["強調色", "Accent"],
  "settings.appearance.theme": ["主題", "Theme"],
  "settings.appearance.theme.system": ["跟隨系統", "Follow system"],
  "settings.appearance.theme.light": ["淺色", "Light"],
  "settings.appearance.theme.dark": ["深色", "Dark"],
  "settings.appearance.theme.now": [
    "目前：{actual}",
    "Current: {actual}",
  ],
  "settings.appearance.autostart": ["開機自啟", "Launch at login"],
  "settings.appearance.autostart.hint": [
    "登入 Windows 時自動啟動（寫 HKCU 的 Run 機碼）",
    "Start on Windows sign-in (HKCU Run key)",
  ],
  "settings.appearance.autostart.on": ["已開啟開機自啟", "Launch at login enabled"],
  "settings.appearance.autostart.off": ["已關閉開機自啟", "Launch at login disabled"],
  "settings.appearance.close": ["關閉視窗時", "On window close"],
  "settings.appearance.close.tray": ["縮到系統匣（建議）", "Minimize to tray (recommended)"],
  "settings.appearance.close.exit": ["直接結束", "Exit completely"],
  "settings.appearance.close.hint": [
    "這是本地網關 —— 縮到系統匣才不會斷掉工具的流量",
    "This is a local gateway — staying in the tray keeps your tools connected",
  ],
  "settings.appearance.close.tray.msg": [
    "關閉視窗時縮到系統匣，網關繼續運作",
    "Closing the window hides it; the gateway keeps running",
  ],
  "settings.appearance.close.exit.msg": [
    "關閉視窗時會直接結束（網關也會停）",
    "Closing the window exits the app (the gateway stops too)",
  ],
  "settings.appearance.lightweight": ["輕量模式", "Lightweight mode"],
  "settings.appearance.lightweight.btn": ["進入輕量模式", "Enter lightweight mode"],
  "settings.appearance.lightweight.hint": [
    "釋放視窗記憶體，托盤與網關照常運作",
    "Frees the window's memory; tray and gateway keep running",
  ],
  "settings.appearance.lightweight.msg": [
    "已進入輕量模式（可從托盤「開啟主視窗」或點 Deep Link 回來）",
    "Lightweight mode on (reopen from the tray or a deep link)",
  ],

  // ── 設定：用量匯出 ──
  "settings.export.title": ["用量匯出", "Usage export"],
  "settings.export.caption": [
    "請求明細 CSV（含 BOM，Excel 可直接開）",
    "Request detail CSV (BOM included, opens in Excel)",
  ],
  "settings.export.range": ["區間", "Range"],
  "settings.export.csv": ["匯出 CSV", "Export CSV"],
  "settings.export.working": ["匯出中…", "Exporting…"],
  "settings.export.done": ["已匯出 {n} 筆", "Exported {n} rows"],
  "settings.export.failed": ["匯出失敗：{e}", "Export failed: {e}"],

  // ── 設定：內建小工具 ──
  "settings.utilities.title": ["內建小工具", "Built-in utilities"],
  "settings.utilities.caption": [
    "對 Claude Code 的設定檔做最小增量修改：只碰一個鍵，其餘原樣保留",
    "Minimal edits to Claude Code's config files: one key touched, everything else preserved",
  ],
  "settings.utilities.onboarding": [
    "跳過初次安裝確認",
    "Skip first-run confirmation",
  ],
  "settings.utilities.onboarding.desc": [
    "寫入 ~/.claude.json 的 hasCompletedOnboarding，開啟時不再問「信任這台機器」",
    "Writes hasCompletedOnboarding to ~/.claude.json so the trust prompt is skipped",
  ],
  "settings.utilities.attribution": ["隱藏 AI 署名", "Hide AI attribution"],
  "settings.utilities.attribution.desc": [
    "把 ~/.claude/settings.json 的 includeCoAuthoredBy 設為 false，commit 不再加 Co-Authored-By",
    "Sets includeCoAuthoredBy=false in ~/.claude/settings.json so commits carry no Co-Authored-By",
  ],
  "settings.utilities.plugin": [
    "編輯器擴充整合",
    "Editor extension integration",
  ],
  "settings.utilities.plugin.desc": [
    "寫入 ~/.claude/config.json 的 primaryApiKey=any，VS Code 擴充不再要求另外貼金鑰",
    "Writes primaryApiKey=any to ~/.claude/config.json so the VS Code extension stops asking for a key",
  ],
  "settings.utilities.hint": [
    "關閉任何一個開關都是「把那個鍵刪掉」，不是寫反向值 —— 回到這個程式沒碰過的樣子。寫入前會走原子寫入（先寫 .tmp 再 rename），JSON 壞掉時直接拒絕、不會覆蓋你的原檔。",
    "Turning a switch off deletes the key (not writes the opposite value), restoring the untouched state. Writes are atomic and invalid JSON is refused rather than overwritten.",
  ],
  "settings.utilities.failed": ["讀取小工具狀態失敗：{e}", "Failed to read utilities: {e}"],

  // ── 設定：Claude Desktop ──
  "settings.desktop.title": ["Claude Desktop", "Claude Desktop"],
  "settings.desktop.caption": [
    "用 Claude 形狀的別名把 Desktop 接到本網關（Desktop 會拒收非 Claude 的模型 ID）",
    "Connect Desktop to this gateway via Claude-shaped aliases (Desktop rejects non-Claude model IDs)",
  ],
  "settings.desktop.state": ["狀態", "Status"],
  "settings.desktop.apply": ["套用", "Apply"],
  "settings.desktop.restore": ["還原", "Restore"],
  "settings.desktop.gateway": ["網關位址", "Gateway URL"],
  "settings.desktop.key": ["使用的金鑰", "Key"],
  "settings.desktop.noKey": ["沒有可用的本地 Key", "No usable local key"],
  "settings.desktop.aliases": ["模型對映（Desktop 送別名，網關改寫成真模型）", "Model mapping (Desktop sends the alias; the gateway rewrites it)"],
  "settings.desktop.noAlias": ["尚未建立（按「套用」會自動補）", "not created yet (Apply creates it)"],
  "settings.desktop.notInstalled": [
    "找不到 Claude Desktop 的 configLibrary —— 本機可能沒安裝",
    "Claude Desktop's configLibrary not found - it may not be installed",
  ],
  "settings.desktop.hint": [
    "會另寫一份自己的 profile（不覆蓋其他工具寫的），只改 _meta.json 的 appliedId；還原時指回原本那份。改完請完全結束並重新開啟 Claude Desktop。",
    "Writes its own profile (never overwriting others) and only changes _meta.json's appliedId; Restore points back to the previous one. Fully quit and reopen Claude Desktop afterwards.",
  ],

  // ── 設定：關於 ──
  "settings.about.title": ["關於", "About"],
  "settings.about.caption": [
    "Token Portal v2 · 本機 AI 用量收費站",
    "Token Portal v2 · local AI toll booth",
  ],
  "settings.about.datadir": ["數據目錄", "Data folder"],
  "settings.about.copy": ["複製", "Copy"],

  // ── 語言 ──
  "settings.lang.title": ["語言", "Language"],
  "settings.lang.caption": [
    "介面語言（切換即時生效，不用重啟）",
    "Interface language (applies immediately, no restart)",
  ],

  "settings.export.range.today": ["今日", "Today"],
  "settings.export.range.7d": ["最近 7 天", "Last 7 days"],
  "settings.export.range.30d": ["最近 30 天", "Last 30 days"],

  // ── 共用 ──
  "common.loading": ["載入中…", "Loading…"],
  "common.readFailed": ["讀取失敗", "Read failed"],
};
