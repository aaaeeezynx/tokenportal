//! 由 `Providers.tsx` 拆分而來（原檔 2,845 行）。程式碼語意未改，只搬位置。

import {
  type ProviderInput,
  type SwitchRequest,
} from "../../lib/api";

export const APP_TYPES = ["universal", "claude", "codex", "opencode", "hermes", "dsh"];

/**
 * 接口格式。**預設是 `auto`**（逐模型偵測）。
 *
 * 為什麼把選項收成這樣（2026-10-10）：協議是「某個模型在某個上游」的屬性，
 * 不是「來源」的屬性 —— 訂閱制的 opencode-go 單一 base_url、一把金鑰底下，
 * `claude-haiku-5-5` 只吃 Anthropic Messages、其餘模型吃 OpenAI chat。
 * 要使用者為此把同一個訂閱拆成兩個來源，代價是金鑰重複，而且 App 的
 * 用量／成本／健康度統計被切碎（上游那邊本來就只有一份額度）。
 *
 * `auto` 會依「客戶端自己用的協議」先試，再換其餘的，並把結果記住；
 * 也可以按「偵測協議與思考強度」一次問清楚。
 *
 * `gemini` 維持明確指定：它的 URL 形狀（模型名在路徑裡）與鑑權都不同，
 * 自動探測不划算。其餘舊值放在 `API_FORMATS_ADVANCED`，想釘死的人仍可用。
 */
export const API_FORMATS = [
  { id: "auto", label: "自動（推薦，逐模型偵測）" },
  { id: "gemini", label: "Gemini（原生）" },
];

/** 進階：明確釘死來源協議（`auto` 猜不出來、或想避免探測成本時用）。 */
export const API_FORMATS_ADVANCED = [
  "auto",
  "openai-chat",
  "openai-responses",
  "mixed",
  "anthropic",
  "gemini",
];
export const AUTH_SCHEMES = [
  { id: "bearer", label: "Bearer（預設）" },
  { id: "goog-key", label: "Google API Key" },
  { id: "anthropic", label: "Anthropic API Key" },
];

export const EMPTY_INPUT: ProviderInput = {
  name: "",
  app_type: "universal",
  // 新來源預設 `auto`：逐模型偵測協議。使用者的痛點是訂閱制上游
  // （一個 base_url、一把金鑰）同時含不同接口格式 —— 舊版只能靠拆來源解決，
  // 代價是金鑰重複與統計被切碎。見 API_FORMATS 的註解。
  api_format: "auto",
  base_url: "",
  api_key: "",
  auth_scheme: "bearer",
  protocol_prefixes: "",
  models: [],
  priority: 0,
  enabled: true,
};

export function copyText(t: string) {
  void navigator.clipboard?.writeText(t).catch(() => {});
}

/** 上次成功切換的參數（行開關一鍵重開用；存本機 localStorage）。 */
export const LAST_KEY = (app: string) => `tg:lastSwitch:v1:${app}`;
export function saveLastSwitch(app: string, req: SwitchRequest) {
  try {
    localStorage.setItem(
      LAST_KEY(app),
      JSON.stringify({ ...req, base_url: "" }),
    );
  } catch {
    /* 忽略配額錯誤 */
  }
}
export function loadLastSwitch(app: string): SwitchRequest | null {
  try {
    const raw = localStorage.getItem(LAST_KEY(app));
    if (!raw) return null;
    const r = JSON.parse(raw) as SwitchRequest;
    if (!r || r.app !== app || !r.model) return null;
    return r;
  } catch {
    return null;
  }
}
