/**
 * 設定頁的「Claude Desktop」區塊。
 *
 * ## 為什麼這個功能不是「填個 base_url」就好
 *
 * Claude Desktop 有 **fail-all validator**：`inferenceModels` 裡只要有一個
 * 不像 Claude 的模型 ID，整組 profile 就會被拒收。所以真正的模型必須藏在
 * Claude 形狀的別名後面（`claude-sonnet-5` 等），由網關在轉發前改寫成
 * `actual_model`。這裡把「四個別名 → 哪個模型」直接列出來，使用者才看得懂
 * 到底發生了什麼。
 *
 * ## 與 CC Switch 共存
 *
 * `configLibrary/` 可以放多份 profile，用 `_meta.json` 的 `appliedId` 決定
 * 哪一份生效。本程式**另寫一份**（自己的 ID），只改 `appliedId`；
 * 「還原」會把自己那條移除並指回原本那份。
 */
import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api, useGatewayPort } from "../../lib/api";
import { useI18n } from "../../lib/i18n";
import { Icon } from "../icons";
import { SectionHead } from "./SectionHead";

export function ClaudeDesktopSection() {
  const { t } = useI18n();
  const qc = useQueryClient();
  const port = useGatewayPort();
  const st = useQuery({
    queryKey: ["desktop", port],
    queryFn: () => api.desktopStatus(port),
  });
  const [msg, setMsg] = useState<string>("");

  const apply = useMutation({
    mutationFn: () => api.desktopApply(port),
    onSuccess: ([s, m]) => {
      qc.setQueryData(["desktop", port], s);
      setMsg(m);
    },
    onError: (e) => setMsg(String(e)),
  });
  const restore = useMutation({
    mutationFn: () => api.desktopRestore(port),
    onSuccess: ([s, m]) => {
      qc.setQueryData(["desktop", port], s);
      setMsg(m);
    },
    onError: (e) => setMsg(String(e)),
  });

  const d = st.data;
  const busy = apply.isPending || restore.isPending;

  return (
    <div className="glass p-5">
      <SectionHead
        icon="cpu"
        tile="linear-gradient(160deg, #f59e0b, #b45309)"
        title={t("settings.desktop.title")}
        caption={t("settings.desktop.caption")}
      />

      {st.isPending ? (
        <p className="py-2 text-xs text-fg/30">{t("common.loading")}</p>
      ) : st.isError ? (
        <p className="py-2 text-xs text-fg/45">{String(st.error)}</p>
      ) : !d?.installed ? (
        <p className="py-2 text-xs text-fg/35">{t("settings.desktop.notInstalled")}</p>
      ) : (
        <>
          <div className="mac-frow">
            <span className="mac-cap">{t("settings.desktop.state")}</span>
            <div className="flex min-w-0 flex-1 items-center gap-2">
              <span
                className={`h-2 w-2 shrink-0 rounded-full ${
                  d.configured ? "bg-emerald-400" : "bg-fg/25"
                }`}
              />
              <span className="min-w-0 flex-1 text-[13px] text-fg/75">{d.note}</span>
              <button
                className="btn-primary flex shrink-0 items-center gap-1.5 px-3.5 py-1.5 text-[13px] disabled:opacity-40"
                disabled={busy || !d.keyName}
                onClick={() => (d.configured ? restore.mutate() : apply.mutate())}
              >
                <Icon name={d.configured ? "x" : "check"} size={12} />
                {d.configured
                  ? t("settings.desktop.restore")
                  : t("settings.desktop.apply")}
              </button>
            </div>
          </div>

          <div className="mac-frow">
            <span className="mac-cap">{t("settings.desktop.gateway")}</span>
            <span className="min-w-0 flex-1 truncate font-mono text-[11px] text-fg/40">
              {d.baseUrl}
            </span>
          </div>
          <div className="mac-frow">
            <span className="mac-cap">{t("settings.desktop.key")}</span>
            <span className="min-w-0 flex-1 truncate text-[12px] text-fg/55">
              {d.keyName ?? t("settings.desktop.noKey")}
            </span>
          </div>

          {/* 四個 Claude 別名的對映 */}
          <p className="pt-2 pb-1 text-[11px] text-fg/30">
            {t("settings.desktop.aliases")}
          </p>
          <div className="space-y-1">
            {d.aliases.map((a) => (
              <div key={a.name} className="flex items-center gap-2 text-[12px]">
                <span className="w-40 shrink-0 font-mono text-fg/60">{a.name}</span>
                <span className="shrink-0 text-fg/20">→</span>
                <span
                  className={`min-w-0 flex-1 truncate font-mono ${
                    a.actual ? "text-fg/50" : "text-(--mac-amber)"
                  }`}
                >
                  {a.actual ?? t("settings.desktop.noAlias")}
                </span>
                {a.provider && (
                  <span className="shrink-0 text-[10px] text-fg/25">{a.provider}</span>
                )}
              </div>
            ))}
          </div>

          <p className="pt-1 truncate font-mono text-[10px] text-fg/20">
            {d.profilePath}
          </p>
        </>
      )}

      {msg && <p className="pt-1.5 text-xs break-all text-fg/50">{msg}</p>}
      <p className="pt-1.5 text-[11px] leading-relaxed text-fg/25">
        {t("settings.desktop.hint")}
      </p>
    </div>
  );
}
