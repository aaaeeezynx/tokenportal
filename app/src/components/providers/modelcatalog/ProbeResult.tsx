/**
 * 思考強度探測的結果顯示（兩個呼叫端共用）。
 *
 * 三種結果分開講，因為它們的意義完全不同：
 * - 支援／不支援 → 已寫入設定與記憶
 * - 無法判定（429／5xx／逾時）→ **不寫入**，避免把一次限流誤記成「不支援」
 * - 整輪中止 → 連基準請求都沒過，這是金鑰／端點／上游的問題，不是強度
 */
import type { ReasoningProbeReport } from "../../../lib/api";

export function ProbeResult(props: { report: ReasoningProbeReport }) {
  const r = props.report;

  if (r.aborted) {
    return (
      <div className="rounded-lg border border-red-500/20 bg-red-500/[0.05] px-3 py-2 text-[11px] break-words text-(--mac-red)">
        無法偵測：{r.aborted}
        <div className="pt-1 text-fg/40">
          這不是強度的問題，所以沒有改動任何設定。修好之後再按一次。
        </div>
      </div>
    );
  }

  return (
    <div className="space-y-1 rounded-lg bg-fg/[0.03] px-3 py-2 text-[11px]">
      <div className="text-fg/40">
        已發 {r.requests} 個請求（1 個基準 ＋ 8 個候選強度）
      </div>
      {r.supported.length > 0 ? (
        <div className="text-(--mac-green)">
          支援：{r.supported.join(" / ")}
        </div>
      ) : (
        <div className="text-(--mac-amber)">
          這個模型不接受任何思考強度（全部被上游拒絕）
        </div>
      )}
      {r.unsupported.length > 0 && (
        <div className="text-(--mac-red)">
          不支援：{r.unsupported.join(" / ")}
        </div>
      )}
      {r.inconclusive.length > 0 && (
        <div className="text-(--mac-amber)">
          無法判定（未寫入）：
          {r.inconclusive.map((o) => ` ${o.level}`).join("、")}
          <div className="text-fg/35">
            多半是限流或上游暫時故障 —— 重按一次即可。
          </div>
        </div>
      )}
      <div className="pt-0.5 text-fg/40">
        已寫入這個模型的思考等級。重新生成目錄後 Codex 才會看到。
      </div>
      {r.inconclusive
        .filter((o) => o.level.endsWith("（樣本）"))
        .map((o) => (
          <div key={o.level} className="font-mono text-[10px] break-all text-fg/25">
            上游原文：{o.detail}
          </div>
        ))}
    </div>
  );
}
