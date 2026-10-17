//! 由 `ModelCatalog.tsx` 拆分而來（原檔 613 行）。程式碼語意未改，只搬位置。

import {
  useState,
} from "react";
import {
  useMutation,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import {
  api,
  REASONING_OPTIONS,
  type Provider,
  type UsableModelInput,
} from "../../../lib/api";
import {
  Icon,
} from "../../icons";
import {
  PopSelect,
} from "../../PopSelect";
import {
  fmtCatalogTime,
} from "./shared";
import {
  ModelRowAcc,
} from "./ModelRowAcc";
import {
  ContextWindowField,
  DEFAULT_CONTEXT_WINDOW,
} from "./ContextWindowField";

/**
 * 批次偵測的一行結果 —— **只有名稱與支援的檔位**。
 *
 * 使用者要的是 `模型名：low / medium / high / max`，不要協議名、也不要
 * 「不支援 …」的清單（那些在單一模型的卡片裡看得到）。
 */
function shortLine(
  name: string,
  rep: { aborted: string | null; supported: string[] },
): string {
  if (rep.aborted) return `${name}：偵測失敗`;
  if (rep.supported.length === 0) return `${name}：不支援思考強度`;
  return `${name}：${rep.supported.join(" / ")}`;
}

export function ModelPanel(props: { provider: Provider; rowsClassName?: string; apiKeyOverride?: string; baseUrlOverride?: string }) {
  const qc = useQueryClient();
  const pid = props.provider.id;
  const catalog = useQuery({
    queryKey: ["catalog", pid],
    queryFn: () => api.catalogGet(pid),
    retry: false,
  });
  const rows = useQuery({
    queryKey: ["umodels", pid],
    queryFn: () => api.modelsList(pid),
  });
  /** 進度條的分母（批次偵測用；`probeDone` 是分子）。 */
  const total = (rows.data || []).length;
  const [adding, setAdding] = useState(false);
  const [actual, setActual] = useState("");
  const [display, setDisplay] = useState("");
  // 預設 256K：明確寫入，不讓「不指定」一路傳到目錄去繼承模板的 1M
  const [context, setContext] = useState(String(DEFAULT_CONTEXT_WINDOW));
  const [reasoning, setReasoning] = useState<string>("");
  const [msg, setMsg] = useState("");
  const [probeDone, setProbeDone] = useState(0);
  const [probeAllReport, setProbeAllReport] = useState<string[]>([]);

  /**
   * 批次偵測：**逐一序列執行**，不並行 —— 並行會撞上游的速率限制，
   * 而且一次限流會被誤判成「這個強度不支援」。前端逐筆呼叫單模型命令，
   * 所以進度與中斷都是自然的（畫面關掉就停）。
   */
  const probeAll = useMutation({
    mutationFn: async () => {
      const list = rows.data ?? [];
      setProbeAllReport([]);
      const out: string[] = [];
      for (let i = 0; i < list.length; i++) {
        // 進度以「已完成」計：開始第 i 個時，前 i 個已經跑完
        setProbeDone(i);
        const r = list[i];
        try {
          // 先協議、後強度：協議不通就沒必要測強度（見 ModelRowAcc 的同名註解）。
          //
          // 2026-10-11：**四種協議都能測了**（探測器改依協議選機制：
          // chat/responses 用 reasoning_effort、anthropic 用 thinking、
          // gemini 用 thinkingConfig），所以不再只跑 chat/responses。
          await api.modelProbeProtocol(pid, r.id);
          const rep = await api.modelProbeReasoning(pid, r.id);
          out.push(shortLine(r.display_name, rep));
        } catch (e) {
          out.push(`${r.display_name}：偵測失敗`);
          void e;
        }
        setProbeDone(i + 1);
        setProbeAllReport([...out]);
      }
      setProbeDone(list.length);
      return out;
    },
    onSuccess: () => {
      refresh();
      setMsg("");
    },
    onError: (e) => setMsg(String(e)),
  });

  const refresh = () => {
    void qc.invalidateQueries({ queryKey: ["umodels", pid] });
    void qc.invalidateQueries({ queryKey: ["model_counts"] });
  };
  const fetch = useMutation({
    mutationFn: () => api.catalogFetch(pid, props.apiKeyOverride, props.baseUrlOverride),
    onSuccess: (c) => {
      setMsg(`已取得 ${c.models.length} 個模型`);
      void qc.invalidateQueries({ queryKey: ["catalog", pid] });
    },
    onError: (e) => setMsg(String(e)),
  });
  const add = useMutation({
    mutationFn: () => {
      const a = actual.trim();
      const input: UsableModelInput = {
        provider_id: pid,
        display_name: display.trim() || a,
        actual_model: a,
        context_window:
          context.trim() === "" ? null : Number(context) || 0,
        reasoning,
        // 新增時留空 = 自動；按「偵測協議與思考強度」會自動填入
        protocol: "",
        ord: (rows.data?.length ?? 0) * 10,
        enabled: true,
      };
      return api.modelAdd(input);
    },
    onSuccess: () => {
      setAdding(false);
      setActual("");
      setDisplay("");
      setContext(String(DEFAULT_CONTEXT_WINDOW));
      setReasoning("");
      setMsg("");
      refresh();
    },
    onError: (e) => setMsg(String(e)),
  });

  const catalogIds = (catalog.data?.models || []).map((m) => m.id);
  const [openId, setOpenId] = useState<number | null>(null);
  const inputClsSm = "field w-full px-3 py-1.5 text-xs";
  return (
    <div>
      <div className="mb-3 flex flex-wrap items-center gap-2">
        <button
          className="btn-ghost flex items-center gap-1.5 px-3.5 py-1.5 text-[13px] disabled:opacity-40"
          disabled={fetch.isPending}
          onClick={() => fetch.mutate()}
        >
          <Icon name="download" size={13} />
          取得模型清單
        </button>
        <button
          className="btn-ghost flex items-center gap-1.5 px-3.5 py-1.5 text-[13px]"
          onClick={() => {
            setActual(catalogIds[0] ?? "");
            setDisplay("");
            setContext("");
            setReasoning("");
            setOpenId(null);
            setAdding(true);
          }}
        >
          <Icon name="plus" size={12} />
          新增模型
        </button>
        <span className="ml-auto text-[11px] text-fg/25">
          {catalog.data
            ? `共 ${catalog.data.models.length} 個 · 更新於 ${fmtCatalogTime(catalog.data.fetched_at)}`
            : "尚未抓取"}
        </span>
      </div>
      <div className="mb-3 flex items-center gap-2">
        <button
          // 尺寸**完全對齊上方「取得模型清單」**：`px-3.5 py-1.5 text-[13px]`。
          // `whitespace-nowrap` 讓「偵測中…」與平常都是同一行高，不會被擠成兩行。
          className="btn-ghost flex shrink-0 items-center gap-1.5 px-3.5 py-1.5 text-[13px] whitespace-nowrap disabled:opacity-40"
          disabled={probeAll.isPending || (rows.data || []).length === 0}
          title="依序實測本來源每個模型支援哪些思考強度，並帶入設定（會對上游發真正的請求）"
          onClick={() => probeAll.mutate()}
        >
          {probeAll.isPending ? "偵測中…" : "偵測思考強度"}
        </button>
        <span className="min-w-0 flex-1 text-[10px] leading-snug text-fg/25">
          約 9 筆請求／模型
        </span>
      </div>
      {probeAll.isPending && (
        <div className="relative mb-3">
          {/*
            與用量頁的訂閱圓環同一套語言（見 `usage/usageQuota.tsx` 的 QuotaRing），
            圓環換成橫條。

            **光暈必須畫在軌道外面**：軌道為了切圓角用了 `overflow-hidden`，
            在那裡面做的 drop-shadow 會被整條裁掉（第一版就是這樣，所以看不出光）。

            三層由下而上：
            1. 光暈暈染 —— 比條身高的模糊色塊，外溢成光
            2. 軌道 —— 半透明，看得出「還剩多少」
            3. 填充 —— 強調色**漸層**（左實右透），透明感才看得出來
          */}
          <div
            className="pointer-events-none absolute left-0 top-1/2 h-3 -translate-y-1/2 rounded-full"
            style={{
              width: `${total > 0 ? (probeDone / total) * 100 : 0}%`,
              background: "var(--mac-accent)",
              filter: "blur(7px)",
              opacity: 0.75,
              transition: "width 0.6s var(--mac-ease)",
            }}
          />
          <div
            className="relative h-1.5 w-full overflow-hidden rounded-full"
            style={{
              background: "rgba(255,255,255,0.13)",
              boxShadow: "inset 0 0 0 0.5px rgba(255,255,255,0.06)",
            }}
            title="偵測進度"
          >
            <div
              className="absolute inset-y-0 left-0 rounded-full"
              style={{
                width: `${total > 0 ? (probeDone / total) * 100 : 0}%`,
                background:
                  "linear-gradient(90deg, color-mix(in srgb, var(--mac-accent) 100%, transparent) 0%, color-mix(in srgb, var(--mac-accent) 45%, transparent) 100%)",
                transition: "width 0.6s var(--mac-ease)",
              }}
            />
          </div>
        </div>
      )}
      {probeAllReport.length > 0 && (
        <div className="mb-2 space-y-1 rounded-lg bg-fg/[0.03] px-3 py-2 text-[11px]">
          {probeAllReport.map((line) => (
            <div key={line} className="break-words text-fg/60">
              {line}
            </div>
          ))}
        </div>
      )}
      {!props.provider.api_key && !props.apiKeyOverride && (
        <p className="mb-2 text-xs text-(--mac-amber)">
          該來源尚未填寫 API Key（請在上方表單填入），否則抓取會 401/403
        </p>
      )}
      {msg && <p className="mb-2 text-xs text-(--mac-amber)">{msg}</p>}
      <div className={`mac-rows scroll-y ${props.rowsClassName ?? "max-h-[38vh]"}`}>
        {(rows.data || []).map((r) => (
          <ModelRowAcc
            key={r.id}
            row={r}
            catalogIds={catalogIds}
            open={openId === r.id}
            onToggle={() => setOpenId((o) => (o === r.id ? null : r.id))}
            onChanged={refresh}
            onMsg={setMsg}
          />
        ))}
        {(rows.data || []).length === 0 && (
          <div className="mac-row text-xs text-fg/20">
            暫無使用模型，點「新增模型」建立（修改後需重啟對應 CLI 生效）。
          </div>
        )}
      </div>
      {adding && (
        <div className="mac-rows mt-2 p-3.5">
          <div className="mb-1 text-[13px] font-semibold text-fg/80">
            新增模型
          </div>
          <div className="mac-frow">
            <span className="mac-cap">選單顯示名</span>
            <input
              className={inputClsSm}
              placeholder="空=同實際模型"
              value={display}
              onChange={(e) => setDisplay(e.target.value)}
            />
          </div>
          <div className="mac-frow">
            <span className="mac-cap">實際請求模型</span>
            {catalogIds.length > 0 ? (
              <PopSelect
                searchable
                value={actual}
                onChange={setActual}
                options={catalogIds.map((id) => ({ value: id, label: id }))}
              />
            ) : (
              <input
                className={inputClsSm}
                placeholder="實際請求模型 ID"
                value={actual}
                onChange={(e) => setActual(e.target.value)}
              />
            )}
          </div>
          <div className="mac-frow">
            <span className="mac-cap">上下文視窗</span>
            <ContextWindowField
              value={context}
              onChange={setContext}
              inputClassName={inputClsSm}
            />
          </div>
          <div className="mac-frow">
            <span className="mac-cap">思考等級<span className="ml-1 font-normal text-fg/25">空 = 全部檔位</span></span>
            <PopSelect
              multi
              value={reasoning}
              onChange={setReasoning}
              options={REASONING_OPTIONS.filter((o) => o.id !== "unset").map((o) => ({
                value: o.id,
                label: o.label,
              }))}
            />
          </div>
          <div className="flex justify-end gap-2 pt-2.5">
            <button
              className="btn-ghost px-4 py-1.5 text-xs"
              onClick={() => setAdding(false)}
            >
              取消
            </button>
            <button
              className="btn-primary px-4 py-1.5 text-xs disabled:opacity-40"
              disabled={add.isPending}
              onClick={() => add.mutate()}
            >
              儲存
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
