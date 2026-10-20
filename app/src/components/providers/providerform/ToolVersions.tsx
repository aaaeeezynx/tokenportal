//! 由 `components/providers/ProviderForm.tsx` 拆分而來。程式碼語意未改，只搬位置。

import {
  useMemo,
  useState,
} from "react";
import {
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import {
  api,
} from "../../../lib/api";
import {
  Icon,
} from "../../icons";
import {
  Logo,
} from "../../logos";

export function ToolVersions() {
  const qc = useQueryClient();
  const vers = useQuery({
    queryKey: ["tool_versions"],
    queryFn: api.toolVersions,
    staleTime: 60_000,
  });
  // 最新版走 registry，慢查詢獨立跑，不擋版本顯示。
  const latestQ = useQuery({
    queryKey: ["tool_latest"],
    queryFn: api.toolLatest,
    staleTime: 5 * 60_000,
    retry: 1,
  });
  const latestMap = useMemo(
    () => new Map((latestQ.data || []).map((l) => [l.app, l] as const)),
    [latestQ.data],
  );
  const [updating, setUpdating] = useState<string | null>(null);
  const [updMsg, setUpdMsg] = useState<{ app: string; ok: boolean; msg: string } | null>(null);
  const [busy, setBusy] = useState<"install" | "all" | null>(null);
  // 重複安裝診斷：同一個工具有兩份時，「跑的那份」可能不是剛更新的那份。
  const dupQ = useQuery({
    queryKey: ["tool_duplicates"],
    queryFn: api.toolDuplicates,
    staleTime: 60_000,
  });
  const dupMap = useMemo(
    () => new Map((dupQ.data || []).map((d) => [d.app, d] as const)),
    [dupQ.data],
  );
  const refreshAll = () => {
    void qc.invalidateQueries({ queryKey: ["tool_versions"] });
    void qc.invalidateQueries({ queryKey: ["tool_latest"] });
    void qc.invalidateQueries({ queryKey: ["tool_duplicates"] });
    void qc.invalidateQueries({ queryKey: ["tools"] });
  };
  const runInstall = async (app: string) => {
    setBusy("install");
    setUpdMsg(null);
    try {
      setUpdMsg({ app, ok: true, msg: await api.toolInstall(app) });
      refreshAll();
    } catch (e) {
      setUpdMsg({ app, ok: false, msg: String(e) });
    } finally {
      setBusy(null);
    }
  };
  const runAll = async () => {
    setBusy("all");
    setUpdMsg(null);
    try {
      const out = await api.toolUpgradeAll();
      const ran = out.filter((o) => o.ran);
      const failed = ran.filter((o) => !o.ok);
      const lines = out.map(
        (o) =>
          `${o.ok ? "✅" : "❌"} ${o.display}：${o.ran ? `${o.from ?? "?"} → ${o.to ?? "?"}` : o.message}`,
      );
      setUpdMsg({
        app: "all",
        ok: failed.length === 0,
        msg: `全部升級完成（真的更新 ${ran.length} 個，失敗 ${failed.length} 個）\n${lines.join("\n")}`,
      });
      refreshAll();
    } catch (e) {
      setUpdMsg({ app: "all", ok: false, msg: String(e) });
    } finally {
      setBusy(null);
    }
  };
  const runUpdate = async (app: string) => {
    setUpdating(app);
    setUpdMsg(null);
    try {
      const msg = await api.toolUpdate(app);
      setUpdMsg({ app, ok: true, msg });
      refreshAll();
    } catch (e) {
      setUpdMsg({ app, ok: false, msg: String(e) });
    } finally {
      setUpdating(null);
    }
  };
  return (
    <div className="space-y-2">
      <div className="flex items-center justify-between gap-2">
        <p className="min-w-0 truncate text-xs text-fg/30">
          版本取自本機實際安裝；最新版查 registry；更新走官方通道，單次最長 10 分鐘
        </p>
        <button
          className="flex shrink-0 items-center gap-1 rounded-full border border-fg/10 px-2.5 py-0.5 text-[11px] text-fg/50 transition-colors hover:border-fg/20 hover:text-fg disabled:opacity-40"
          disabled={vers.isFetching || latestQ.isFetching}
          onClick={() => {
            void vers.refetch();
            void latestQ.refetch();
            void dupQ.refetch();
          }}
        >
          <Icon name="refresh" size={12} />
          {vers.isFetching || latestQ.isFetching ? "刷新中…" : "刷新"}
        </button>
      </div>
      <div className="flex items-center justify-between gap-2">
        <p className="min-w-0 truncate text-xs text-fg/30">
          {(() => {
            const behind = (vers.data || []).filter((v) => {
              const l = latestMap.get(v.app);
              return v.installed && (l?.up_to_date ?? v.up_to_date) === false;
            }).length;
            return behind > 0
              ? `${behind} 個工具有新版本`
              : "所有已安裝的工具都是最新";
          })()}
        </p>
        <button
          className="flex shrink-0 items-center gap-1 rounded-full border border-fg/10 px-2.5 py-0.5 text-[11px] text-fg/50 transition-colors hover:border-fg/20 hover:text-fg disabled:opacity-40"
          title="只會升級「已安裝且確實落後」的工具；查不到最新版的一律略過"
          disabled={busy !== null || updating !== null}
          onClick={() => void runAll()}
        >
          <Icon name="upload" size={12} />
          {busy === "all" ? "升級中…" : "全部升級"}
        </button>
      </div>
      {vers.isPending && <p className="text-sm text-fg/30">讀取版本中…</p>}
      {vers.isError && (
        <p className="text-sm text-(--mac-red)">讀取失敗：{String(vers.error)}</p>
      )}
      {(vers.data || []).map((v) => {
        // 最新態：用 registry 比對；比不到＝檢查中。
        const l = latestMap.get(v.app);
        const upToDate = l?.up_to_date ?? v.up_to_date ?? null;
        const latest = l?.latest ?? null;
        const showUpdate =
          v.installed && v.update_argv.length > 0 && upToDate !== true;
        const dup = dupMap.get(v.app);
        return (
          <div
            key={v.app}
            className="rounded-xl border border-fg/[0.06] p-3 text-sm"
          >
          <div className="flex items-center gap-2">
            <Logo name={v.app} size={17} className="shrink-0" />
            <span className="min-w-0 flex-1 truncate font-semibold text-fg/85">
              {v.display}
            </span>
            {!v.installed ? (
              <span className="flex shrink-0 items-center gap-1.5">
                <span className="text-xs text-fg/20">未安裝</span>
                <button
                  className="flex items-center gap-1 rounded-full border border-fg/10 px-2.5 py-0.5 text-[11px] text-fg/50 transition-colors hover:border-fg/20 hover:text-fg disabled:opacity-40"
                  title="npm i -g（全域安裝）"
                  disabled={busy !== null || updating !== null}
                  onClick={() => void runInstall(v.app)}
                >
                  <Icon name="download" size={12} />
                  {busy === "install" ? "安裝中…" : "安裝"}
                </button>
              </span>
            ) : upToDate === true ? (
              <span className="flex shrink-0 items-center gap-1.5">
                <span className="font-mono text-xs text-(--mac-green)">
                  {v.version ?? "未知版本"}
                </span>
                <span className="rounded-full bg-(--mac-green)/10 px-2 py-0.5 text-[10px] font-medium text-(--mac-green)">
                  已是最新
                </span>
              </span>
            ) : (
              <span className="shrink-0 font-mono text-xs text-fg/45">
                {v.version ?? "未知版本"}
                {upToDate === false && latest != null && (
                  <span className="text-(--mac-amber)"> → {latest}</span>
                )}
                {upToDate == null && (
                  <span className="text-fg/25">
                    {latestQ.isPending || latestQ.isFetching ? "（檢查中…）" : "（未查到最新版）"}
                  </span>
                )}
              </span>
            )}
            {showUpdate && (
              <button
                className="flex shrink-0 items-center gap-1 rounded-full border border-fg/10 px-2.5 py-0.5 text-[11px] text-fg/50 transition-colors hover:border-fg/20 hover:text-fg disabled:opacity-40"
                title={v.update_label ?? "更新"}
                disabled={updating === v.app || busy !== null}
                onClick={() => void runUpdate(v.app)}
              >
                <Icon name="download" size={12} />
                {updating === v.app ? "更新中…" : "更新"}
              </button>
            )}
          </div>
          {/* 重複安裝：同一個工具在不同目錄各有一份時，跑的那份可能不是剛更新的那份 */}
          {dup?.duplicated && (
            <div className="mt-1.5 rounded-lg bg-(--mac-amber)/10 px-2.5 py-1.5">
              <p className="text-[11px] text-(--mac-amber)">
                ⚠️ 找到 {dup.copies.length} 份安裝；實際執行的是標示「使用中」那一份
              </p>
              <ul className="mt-1 space-y-0.5 font-mono text-[10px] text-fg/35">
                {dup.copies.map((c) => (
                  <li key={c.path} className={c.active ? "text-(--mac-amber)" : ""}>
                    {c.active ? "▶ " : "　"}
                    {c.path}
                  </li>
                ))}
              </ul>
              <p className="mt-1 text-[10px] text-fg/25">
                更新只會動 npm 全域那一份；若使用中的是別份，請移除多餘的或調整 PATH 順序。
              </p>
            </div>
          )}
        </div>
        );
      })}
      {updMsg && (
        <p
          className={`mt-1.5 text-xs break-all whitespace-pre-wrap ${
            updMsg.ok ? "text-fg/45" : "text-(--mac-red)"
          }`}
        >
          {updMsg.msg}
        </p>
      )}
    </div>
  );
}
