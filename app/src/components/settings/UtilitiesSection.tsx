/**
 * 設定頁的「內建小工具」區塊（對標 CC Switch 的 built-in utilities）。
 *
 * 三個開關都是「對 Claude Code 的設定檔做最小增量修改」：只碰一個鍵，
 * 其餘欄位原樣保留；關閉＝把鍵刪掉（回到「沒碰過」的狀態），不是寫反向值。
 * 後端在 `src-tauri/src/utilities.rs`，那裡有完整的取捨說明。
 *
 * UI 只做兩件事：把三個開關的目前狀態讀出來、以及顯示「會改哪個檔」——
 * 使用者要能一眼看出這個開關動的是哪個檔案，否則不敢開。
 */
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "../../lib/api";
import { useI18n } from "../../lib/i18n";
import { Toggle } from "../Toggle";
import { SectionHead } from "./SectionHead";

/** 三個小工具：key、標題 i18n、說明 i18n、對應的 state 欄位。 */
const ROWS = [
  {
    key: "skipOnboarding",
    field: "skipOnboarding",
    title: "settings.utilities.onboarding",
    desc: "settings.utilities.onboarding.desc",
  },
  {
    key: "hideAttribution",
    field: "hideAttribution",
    title: "settings.utilities.attribution",
    desc: "settings.utilities.attribution.desc",
  },
  {
    key: "pluginIntegration",
    field: "pluginIntegration",
    title: "settings.utilities.plugin",
    desc: "settings.utilities.plugin.desc",
  },
] as const;

export function UtilitiesSection() {
  const { t } = useI18n();
  const qc = useQueryClient();
  const st = useQuery({
    queryKey: ["utilities"],
    queryFn: api.utilitiesState,
  });

  const set = useMutation({
    mutationFn: (v: { key: string; enabled: boolean }) =>
      api.utilitiesSet(v.key, v.enabled),
    onSuccess: (data) => {
      qc.setQueryData(["utilities"], data);
    },
  });

  const targets = st.data?.targets ?? [];
  const pathOf = (key: string) =>
    targets.find((x) => x.key === key)?.path ?? "";

  return (
    <div className="glass p-5">
      <SectionHead
        icon="sliders"
        tile="linear-gradient(160deg, #34d399, #047857)"
        title={t("settings.utilities.title")}
        caption={t("settings.utilities.caption")}
      />

      {st.isPending ? (
        <p className="py-2 text-xs text-fg/30">{t("common.loading")}</p>
      ) : st.isError ? (
        <p className="py-2 text-xs text-fg/45">
          {t("settings.utilities.failed", { e: String(st.error) })}
        </p>
      ) : (
        ROWS.map((r) => (
          <div key={r.key} className="mac-frow items-start">
            <div className="min-w-0 flex-1">
              <span className="mac-cap">{t(r.title)}</span>
              <p className="pt-0.5 text-[11px] leading-snug text-fg/35">
                {t(r.desc)}
              </p>
              <p className="truncate pt-1 font-mono text-[10px] text-fg/20">
                {pathOf(r.key)}
              </p>
            </div>
            <Toggle
              checked={!!st.data?.[r.field]}
              onChange={(v) => set.mutate({ key: r.key, enabled: v })}
              size="sm"
            />
          </div>
        ))
      )}

      {set.isError && (
        <p className="pt-1 text-xs text-red-400/80">{String(set.error)}</p>
      )}
      <p className="pt-2 text-[11px] leading-relaxed text-fg/25">
        {t("settings.utilities.hint")}
      </p>
    </div>
  );
}
