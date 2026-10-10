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
        setProbeDone(i);
        const r = list[i];
        try {
          // 先協議、後強度：協議不通就沒必要測強度（見 ModelRowAcc 的同名註解）
          const proto = await api.modelProbeProtocol(pid, r.id);
          if (proto.protocol !== "chat" && proto.protocol !== "responses") {
            out.push(
              `${r.display_name}：協議 ${proto.protocol ?? "三種都不通"}` +
                (proto.protocol === "anthropic"
                  ? "（只走 Anthropic，Codex 到不了）"
                  : "（請確認模型名與金鑰）"),
            );
            setProbeAllReport([...out]);
            continue;
          }
          const rep = await api.modelProbeReasoning(pid, r.id);
          out.push(
            `${r.display_name}：${proto.protocol}　` +
              (rep.aborted
                ? `失敗（${rep.aborted}）`
                : `${rep.supported.join(" / ") || "（無支援強度）"}` +
                  (rep.unsupported.length
                    ? `　不支援 ${rep.unsupported.join(" / ")}`
                    : "")),
          );
        } catch (e) {
          out.push(`${r.display_name}：${String(e)}`);
        }
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
      <div className="mb-2 flex flex-wrap items-center gap-2">
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
      <div className="mb-2 flex items-center gap-2">
        <button
          className="btn-ghost px-3 py-1 text-[11px] disabled:opacity-40"
          disabled={probeAll.isPending || (rows.data || []).length === 0}
          title="依序實測本來源每個模型支援哪些思考強度，並帶入設定（會對上游發真正的請求）"
          onClick={() => probeAll.mutate()}
        >
          {probeAll.isPending ? `偵測中… ${probeDone}/${(rows.data || []).length}` : "偵測全部模型的思考強度"}
        </button>
        <span className="text-[11px] text-fg/30">
          逐一實測，約 9 筆請求／模型；只改動思考等級，不動其他設定
        </span>
      </div>
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
        <p className="mb-2 text-xs text-amber-400">
          該來源尚未填寫 API Key（請在上方表單填入），否則抓取會 401/403
        </p>
      )}
      {msg && <p className="mb-2 text-xs text-amber-300">{msg}</p>}
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
