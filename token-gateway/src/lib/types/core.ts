/** 核心型別（來源、模型、金鑰、用量、設定）。由 `apiTypes.ts` 轉出。 */

// 型別／常數宣告：核心資料模型（自 api.ts 原樣搬移，未改動任何內容）。
// 其餘型別分置 apiTypesPricing.ts / apiTypesDiagnostics.ts，於此檔尾 re-export。

export interface DbStatus {
  path: string;
  schema_version: number;
  provider_count: number;
  key_count: number;
  log_count: number;
}

export interface ProxyStatus {
  running: boolean;
  listen: string;
  port: number;
  started_at: number | null;
}

export interface Provider {
  id: number;
  name: string;
  app_type: string;
  api_format: string;
  base_url: string;
  api_key: string;
  /** bearer（預設）| goog-key（Google x-goog-api-key） */
  auth_scheme: string;
  /**
   * 逐協議路徑前綴的 JSON（空 = 沒有）。
   *
   * 有些上游把協議做在**路徑前綴**裡（實測 OpenCode Go：同一個 base 底下
   * `/openai/v1/…`、`/anthropic/v1/…`、`/google/v1beta/…`）。有了它，
   * **一個來源能服務全部四種協議**，不必為每種協議各開一個來源。
   */
  protocol_prefixes: string;
  models: string[];
  priority: number;
  enabled: boolean;
}

/** 一個協議的佈局探測結果（見後端 `models/layout.rs`）。 */
export interface ProtocolRoute {
  protocol: string;
  /** 採用的路徑前綴（空 = 沒有前綴） */
  prefix: string;
  /** 最後一次嘗試的 HTTP 狀態（0 = 連線失敗） */
  status: number;
  /** 路由存在嗎（**非 404** 就算存在，與模型支不支援無關） */
  route_exists: boolean;
}

/** 連線佈局偵測報告。 */
export interface LayoutReport {
  /** 最終採用的 base（可能被換成官方記載的那個） */
  base_url: string;
  /** base 是否被改動 */
  base_changed: boolean;
  /** 可直接存進設定的前綴 JSON（空 = 不需要前綴） */
  prefixes: string;
  routes: ProtocolRoute[];
  /** 有路由可達的協議 */
  reachable: string[];
  /** 給人看的一句話 */
  message: string;
}

export interface ProviderInput {
  name: string;
  app_type: string;
  api_format: string;
  base_url: string;
  api_key: string;
  auth_scheme: string;
  /**
   * 逐協議路徑前綴的 JSON（空 = 沒有）。
   *
   * 有些上游把協議做在**路徑前綴**裡（實測 OpenCode Go：同一個 base 底下
   * `/openai/v1/…`、`/anthropic/v1/…`、`/google/v1beta/…`）。有了它，
   * **一個來源能服務全部四種協議**，不必為每種協議各開一個來源。
   */
  protocol_prefixes: string;
  models: string[];
  priority: number;
  enabled: boolean;
}

/**
 * 來源預設（對齊 CC Switch 的 provider presets）。
 *
 * 只帶**連線資訊**，刻意不含模型清單：模型名變動快，一律用表單上的
 * 「取得模型清單」對上游現場抓（`catalog_fetch`）。
 */
export interface Preset {
  id: string;
  name: string;
  base_url: string;
  api_format: string;
  auth_scheme: string;
  app_type: string;
  /** 逐協議路徑前綴的 JSON（只有需要的預設才有） */
  protocol_prefixes: string;
  /** 空字串＝沒有特別提醒 */
  note: string;
}

/** 來源匯入的結果回報（skip 語意：已存在的 `(name, base_url)` 不覆蓋）。 */
export interface ImportReport {
  added: number;
  skipped: number;
  models: number;
  pricing: number;
  periods: number;
  names: string[];
  /** 匯入檔是否帶金鑰（要提醒使用者） */
  includes_keys: boolean;
}

/**
 * 來源連線檢查的結果（P1.5，對標 CC Switch 的 connectivity check）。
 *
 * `reachable` 是**傳輸層**的判斷：401／403／404 都算可達 ——
 * 它們證明主機活著，只是沒有金鑰或那個路徑沒有端點。
 * 「檢查過了」不等於「請求一定成功」（金鑰與模型名是另一回事）。
 */
export interface CheckResult {
  url: string;
  reachable: boolean;
  /** 0 = 連線階段就失敗 */
  status: number;
  latency_ms: number;
  message: string;
}

/** 故障轉移佇列裡的一列（＝某個模型在某個來源上的候選順序）。 */
export interface QueueRow {
  provider_id: number;
  provider_name: string;
  api_format: string;
  enabled: boolean;
  model_count: number;
  /** 這個來源對這個模型學到的協議（null = 還沒學到） */
  learned_format: string | null;
  /** 斷路器：冷卻中（會被排到候選最後） */
  open: boolean;
  cooldown_secs: number;
}

/** 可挑選的模型 + 有幾個來源登記它。 */
export interface QueueModel {
  model: string;
  sources: number;
}
