// 型別／常數宣告：計價、模型目錄與歷史匯入（自 api.ts 原樣搬移，未改動任何內容）。

export interface Pricing {
  model_norm: string;
  in_pm: number;
  out_pm: number;
  cache_read_pm: number;
  cache_create_pm: number;
  source: string;
}

export interface PricingInput {
  model_norm: string;
  in_pm: number;
  out_pm: number;
  cache_read_pm: number;
  cache_create_pm: number;
}

export type PricingMode = "usage" | "subscription" | "tou";

export interface ProviderPricing {
  provider_id: number;
  provider_name: string;
  model_norm: string;
  mode: PricingMode;
  in_pm: number;
  out_pm: number;
  cache_read_pm: number;
  cache_create_pm: number;
  sub_fee_usd: number;
  sub_included_tokens: number;
  source: string;
  used_this_month: number;
  periods: PricingPeriod[];
}

export interface ProviderPricingInput {
  provider_id: number;
  model_norm: string;
  mode: PricingMode;
  in_pm: number;
  out_pm: number;
  cache_read_pm: number;
  cache_create_pm: number;
  sub_fee_usd: number;
  sub_included_tokens: number;
}

export interface PricingPeriod {
  id: number;
  provider_id: number;
  model_norm: string;
  name: string;
  start: string;
  end: string;
  ord: number;
  in_pm: number;
  out_pm: number;
  cache_read_pm: number;
  cache_create_pm: number;
}

export interface PricingPeriodInput {
  provider_id: number;
  model_norm: string;
  name: string;
  start: string;
  end: string;
  ord: number;
  in_pm: number;
  out_pm: number;
  cache_read_pm: number;
  cache_create_pm: number;
}

export type ReasoningLevel =
  | "unset"
  | "minimal"
  | "low"
  | "medium"
  | "high"
  | "xhigh"
  | "max"
  | "ultra"
  | "persistent";

export const REASONING_OPTIONS: { id: ReasoningLevel; label: string }[] = [
  { id: "unset", label: "未設定" },
  { id: "minimal", label: "minimal" },
  { id: "low", label: "low" },
  { id: "medium", label: "medium" },
  { id: "high", label: "high" },
  { id: "xhigh", label: "xhigh" },
  { id: "max", label: "max" },
  // Codex 生態真實存在的兩檔；實測會被送到上游（2026-10-08 補）
  { id: "ultra", label: "ultra" },
  { id: "persistent", label: "persistent" },
];

/** 思考強度探測的報告（見後端 `models/probe.rs`）。 */
export interface ReasoningProbeReport {
  model: string;
  /** 基準請求（不帶強度）是否通過；沒通過就不會測任何候選 */
  baseline_ok: boolean;
  /** 整輪中止的原因（人話） */
  aborted: string | null;
  supported: string[];
  unsupported: string[];
  /** 拿不到明確答案的（429／5xx／逾時）——不會寫入記憶 */
  inconclusive: { level: string; supported: boolean; detail: string }[];
  requests: number;
  checked_at: number;
}

/** 一筆「這個模型的這個強度上游收不收」的記憶。 */
export interface ReasoningFact {
  level: string;
  supported: boolean;
  /** `probe`（實測，權威）或 `learned`（從真實失敗順手學到） */
  source: string;
  checked_at: number;
}

/** 一次協議嘗試（見後端 `models/protocol_probe.rs`）。 */
export interface ProtocolAttempt {
  /** `chat` / `responses` / `anthropic` */
  protocol: string;
  status: number;
  detail: string;
}

/**
 * 協議探測報告。
 *
 * 為什麼要測這個：同一個來源可以**逐模型**混用協議 —— 實測 opencode-go 的
 * `claude-haiku-5-5` 只吃 Anthropic Messages，同來源的 deepseek 吃 OpenAI chat。
 * 但接口格式是**來源級**設定，表達不了這件事，於是目錄會把一個 Codex
 * 永遠到不了的模型也提供出去。
 */
export interface ProtocolProbeReport {
  model: string;
  /** 通的協議；三種都不通就是 null */
  protocol: string | null;
  attempts: ProtocolAttempt[];
  requests: number;
  checked_at: number;
  /** 給人看的一句話（後端組好，前端直接顯示，免得兩邊各寫一份文案） */
  summary: string;
}

export interface PriceExtractResult {
  in_pm: number;
  out_pm: number;
  cache_read_pm: number;
  cache_create_pm: number;
  currency: string;
  confidence: number;
  evidence: string;
  used_model: string;
  source_note: string;
}

export interface CatalogEntry {
  id: string;
}

export interface CatalogCache {
  provider_id: number;
  fetched_at: number;
  models: CatalogEntry[];
}

export interface UsableModel {
  id: number;
  provider_id: number;
  display_name: string;
  actual_model: string;
  context_window: number | null;
  reasoning: string;
  ord: number;
  enabled: boolean;
}

export interface UsableModelInput {
  provider_id: number;
  display_name: string;
  actual_model: string;
  context_window: number | null;
  reasoning: string;
  ord: number;
  enabled: boolean;
}

export interface HistoryScanTool {
  tool: string;
  files: number;
  bytes: number;
  sessions: number;
}

export interface HistoryToolStat {
  tool: string;
  files: number;
  rows: number;
  tokens: number;
  cost_usd: number;
}

export interface HistoryUnpriced {
  model: string;
  tokens: number;
}

export interface HistorySummary {
  files_scanned: number;
  files_imported: number;
  files_unchanged: number;
  rows_added: number;
  dupes_skipped: number;
  empty_skipped: number;
  bad_lines: number;
  in_tok: number;
  out_tok: number;
  cost_usd: number;
  per_tool: HistoryToolStat[];
  unpriced_top: HistoryUnpriced[];
}
