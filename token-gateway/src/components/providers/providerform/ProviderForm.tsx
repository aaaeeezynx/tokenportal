//! 由 `components/providers/ProviderForm.tsx` 拆分而來。程式碼語意未改，只搬位置。

import {
  useState,
} from "react";
import {
  useMutation,
  useQueryClient,
} from "@tanstack/react-query";
import {
  api,
  type Preset,
  type Provider,
  type ProviderInput,
} from "../../../lib/api";
import {
  Icon,
} from "../../icons";
import {
  PopSelect,
} from "../../PopSelect";
import {
  ModelPanel,
} from "../ModelCatalog";
import {
  API_FORMATS,
  API_FORMATS_ADVANCED,
  APP_TYPES,
  AUTH_SCHEMES,
  EMPTY_INPUT,
} from "../providersTypes";

/** 從 `protocol_prefixes` 的 JSON 取某個協議的前綴（壞掉回空字串）。 */
function prefixOf(json: string, proto: string): string {
  try {
    const o = JSON.parse(json || "{}") as Record<string, unknown>;
    return typeof o?.[proto] === "string" ? (o[proto] as string) : "";
  } catch {
    return "";
  }
}

/** 寫回某個協議的前綴；空字串就把鍵刪掉，全空則存空字串（＝沒有前綴）。 */
function withPrefix(json: string, proto: string, value: string): string {
  let o: Record<string, string> = {};
  try {
    const parsed = JSON.parse(json || "{}") as unknown;
    if (parsed && typeof parsed === "object") o = parsed as Record<string, string>;
  } catch {
    o = {};
  }
  const v = value.trim();
  if (v) o[proto] = v;
  else delete o[proto];
  return Object.keys(o).length === 0 ? "" : JSON.stringify(o);
}

export function ProviderForm(props: {
  initial: Provider | null;
  /** 新建時可從預設開始（只填連線資訊，金鑰與模型仍由使用者處理）。 */
  preset?: Preset | null;
  onDone: () => void;
  onCancel: () => void;
}) {
  const qc = useQueryClient();
  const [form, setForm] = useState<ProviderInput>(() =>
    props.initial
      ? {
          name: props.initial.name,
          app_type: props.initial.app_type,
          api_format: props.initial.api_format,
          base_url: props.initial.base_url,
          api_key: props.initial.api_key,
          auth_scheme: props.initial.auth_scheme || "bearer",
          protocol_prefixes: props.initial.protocol_prefixes ?? "",
          models: props.initial.models,
          priority: props.initial.priority,
          enabled: props.initial.enabled,
        }
      : props.preset
        ? {
            ...EMPTY_INPUT,
            name: props.preset.name,
            app_type: props.preset.app_type || "universal",
            api_format: props.preset.api_format,
            base_url: props.preset.base_url,
            auth_scheme: props.preset.auth_scheme || "bearer",
            protocol_prefixes: props.preset.protocol_prefixes ?? "",
          }
        : EMPTY_INPUT,
  );
  const [err, setErr] = useState("");
  const [showKey, setShowKey] = useState(false);
  // 新建模式：本地草稿模型（直接顯示，無需先儲存）
  const [draftModels, setDraftModels] = useState<string[]>([]);
  const [draftInput, setDraftInput] = useState("");
  const [draftFetching, setDraftFetching] = useState(false);
  const [draftMsg, setDraftMsg] = useState("");
  const [created, setCreated] = useState<Provider | null>(null);
  const eff = created ?? props.initial;

  const save = useMutation({
    mutationFn: () => {
      const input: ProviderInput = {
        ...form,
        models: eff ? eff.models : [],
      };
      return eff
        ? api.providerUpdate(eff.id, input)
        : api.providerAdd(input);
    },
    onSuccess: async (p) => {
      void qc.invalidateQueries({ queryKey: ["providers"] });
      if (!eff && draftModels.length > 0) {
        // 將草稿模型批量寫入新建來源
        for (let i = 0; i < draftModels.length; i++) {
          const m = draftModels[i].trim();
          if (!m) continue;
          try {
            await api.modelAdd({
              provider_id: p.id,
              display_name: m,
              actual_model: m,
              context_window: null,
              reasoning: "unset",
              protocol: "",
              ord: i * 10,
              enabled: true,
            });
          } catch {}
        }
        void qc.invalidateQueries({ queryKey: ["umodels", p.id] });
        void qc.invalidateQueries({ queryKey: ["model_counts"] });
      }
      if (eff) {
        props.onDone();
      } else {
        setCreated(p);
        setDraftModels([]);
      }
    },
    onError: (e) => setErr(String(e)),
  });

  const handleDraftFetch = async () => {
    if (!form.name.trim()) {
      setErr("請先填寫來源名稱");
      return;
    }
    if (!form.base_url.trim()) {
      setErr("請先填寫上游地址");
      return;
    }
    if (!form.api_key.trim()) {
      setErr("請先填寫 API Key");
      return;
    }
    setDraftFetching(true);
    setDraftMsg("");
    try {
      const p = await api.providerAdd({
        ...form,
        models: [],
      });
      for (let i = 0; i < draftModels.length; i++) {
        const m = draftModels[i].trim();
        if (!m) continue;
        try {
          await api.modelAdd({
            provider_id: p.id,
            display_name: m,
            actual_model: m,
            context_window: null,
            reasoning: "unset",
            protocol: "",
            ord: i * 10,
            enabled: true,
          });
        } catch {}
      }
      setCreated(p);
      setDraftModels([]);
      setDraftInput("");
      void qc.invalidateQueries({ queryKey: ["providers"] });
      void qc.invalidateQueries({ queryKey: ["model_counts"] });
      try {
        await api.catalogFetch(p.id, form.api_key);
        void qc.invalidateQueries({ queryKey: ["catalog", p.id] });
        setDraftMsg("已建立來源並取得模型清單");
      } catch (e) {
        setDraftMsg(String(e));
      }
    } catch (e) {
      setErr(String(e));
    } finally {
      setDraftFetching(false);
    }
  };

  const set = <K extends keyof ProviderInput>(k: K, v: ProviderInput[K]) =>
    setForm((f) => ({ ...f, [k]: v }));

  const fieldCls = "field w-full px-3 py-1.5 text-[13px]";
  return (
    <div className="space-y-4">
      <div className="form-section">
        <div className="mb-1 text-[11px] font-semibold tracking-wider text-fg/40 uppercase">
          基本資訊
        </div>
        <div className="mac-frow">
          <span className="mac-cap">來源名稱</span>
          <input
            className={fieldCls}
            placeholder="如 ChatGPT"
            value={form.name}
            onChange={(e) => set("name", e.target.value)}
          />
        </div>
        <div className="mac-frow">
          <span className="mac-cap">類型</span>
          <PopSelect
            value={form.app_type}
            onChange={(v) => set("app_type", v)}
            options={APP_TYPES.map((a) => ({ value: a, label: a }))}
          />
        </div>
        <div className="mac-frow">
          <span className="mac-cap">
            接口格式
            <span className="ml-1 font-normal text-fg/25">
              auto = 逐模型偵測（同一個訂閱可混用協議）
            </span>
          </span>
          <PopSelect
            value={form.api_format}
            onChange={(v) => set("api_format", v)}
            options={[
              ...API_FORMATS.map((o) => ({ value: o.id, label: o.label })),
              // 現有的明確值不在簡化清單裡時，補一項讓它顯示得出來（不靜默改掉
              // 使用者的設定）；要改回 auto 就從上面那兩項挑。
              ...(API_FORMATS.some((o) => o.id === form.api_format)
                ? []
                : [{ value: form.api_format, label: `${form.api_format}（目前值）` }]),
            ]}
          />
        </div>
        <details className="mac-frow">
          <summary className="cursor-pointer text-[11px] text-fg/30">
            進階：明確指定來源協議（不看逐模型偵測）
          </summary>
          <PopSelect
            value={form.api_format}
            onChange={(v) => set("api_format", v)}
            options={API_FORMATS_ADVANCED.map((a) => ({ value: a, label: a }))}
          />
        </details>
        <details className="mac-frow">
          <summary className="cursor-pointer text-[11px] text-fg/30">
            進階：逐協議路徑前綴（一個來源服務多種協議）
          </summary>
          <div className="space-y-1.5 pt-1.5">
            <p className="text-[11px] leading-relaxed text-fg/35">
              有些上游把協議做在路徑前綴裡（例：OpenCode Go 同一個 base 底下
              <code className="mx-1">/openai/v1</code>、
              <code className="mx-1">/anthropic/v1</code>、
              <code className="mx-1">/google/v1beta</code>）。
              填了就能用一個來源服務全部四種協議；留空＝用預設。
            </p>
            {(
              [
                ["openai-chat", "Chat"],
                ["openai-responses", "Responses"],
                ["anthropic", "Anthropic"],
                ["gemini", "Gemini"],
              ] as const
            ).map(([proto, label]) => (
              <div key={proto} className="flex items-center gap-2">
                <span className="w-24 shrink-0 text-[11px] text-fg/50">{label}</span>
                <input
                  className={fieldCls}
                  placeholder={
                    proto === "gemini" ? "/google/v1beta" : "/openai/v1"
                  }
                  value={prefixOf(form.protocol_prefixes, proto)}
                  onChange={(e) =>
                    set(
                      "protocol_prefixes",
                      withPrefix(form.protocol_prefixes, proto, e.target.value),
                    )
                  }
                />
              </div>
            ))}
          </div>
        </details>
      </div>

      <div className="form-section">
        <div className="mb-1 text-[11px] font-semibold tracking-wider text-fg/40 uppercase">
          連線
        </div>
        <div className="mac-frow">
          <span className="mac-cap">上游地址</span>
          <input
            className={fieldCls}
            placeholder="base_url，如 https://api.openai.com/v1"
            value={form.base_url}
            onChange={(e) => set("base_url", e.target.value)}
          />
        </div>
        <div className="mac-frow">
          <span className="mac-cap">鑑權方式</span>
          <PopSelect
            value={form.auth_scheme}
            onChange={(v) => set("auth_scheme", v)}
            options={AUTH_SCHEMES.map((a) => ({
              value: a.id,
              label: a.label,
            }))}
          />
        </div>
        <div className="mac-frow">
          <span className="mac-cap">API Key</span>
          <div className="relative">
            <input
              className={`${fieldCls} pr-9`}
              type={showKey ? "text" : "password"}
              autoComplete="new-password"
              spellCheck={false}
              placeholder="明文存本地庫，M3 加固"
              value={form.api_key}
              onChange={(e) => set("api_key", e.target.value)}
            />
            <button
              type="button"
              className="absolute top-1/2 right-2 -translate-y-1/2 rounded p-1 text-fg/30 transition-colors hover:text-fg/70"
              title={showKey ? "隱藏" : "顯示"}
              onClick={() => setShowKey((v) => !v)}
            >
              <Icon name={showKey ? "eye-slash" : "eye"} size={15} />
            </button>
          </div>
        </div>
      </div>

      <div className="form-section">
        <div className="mb-1 flex items-center gap-2 text-[11px] font-semibold tracking-wider text-fg/40 uppercase">
          <span>使用模型</span>
          {!eff && draftModels.length > 0 && (
            <span className="ml-auto rounded-full bg-fg/[0.08] px-2 py-0.5 text-[10px] font-medium text-fg/50">
              {draftModels.length}
            </span>
          )}
        </div>
        {eff ? (
          <ModelPanel
            key={eff.id}
            provider={eff}
            rowsClassName="max-h-[38vh]"
            apiKeyOverride={form.api_key}
            baseUrlOverride={form.base_url}
          />
        ) : (
          <div className="space-y-2">
            <div className="flex items-center gap-2">
              <button
                type="button"
                className="btn-ghost flex items-center gap-1.5 px-3 py-1.5 text-xs disabled:opacity-40"
                disabled={draftFetching}
                onClick={() => void handleDraftFetch()}
              >
                <Icon name="download" size={11} />
                {draftFetching ? "建立中…" : "取得模型清單"}
              </button>
              <span className="text-[11px] text-fg/25">貼上 API Key 後直接抓取，無需先點儲存</span>
            </div>
            {draftMsg && <p className="text-xs text-(--mac-amber)">{draftMsg}</p>}
            {draftModels.length > 0 && (
              <div className="flex flex-wrap gap-1.5">
                {draftModels.map((m) => (
                  <span
                    key={m}
                    className="flex items-center gap-1 rounded-full bg-fg/[0.07] px-2.5 py-1 text-xs text-fg/70"
                  >
                    <span className="font-mono">{m}</span>
                    <button
                      type="button"
                      className="rounded-full p-0.5 text-fg/40 hover:bg-fg/10 hover:text-fg"
                      onClick={() => setDraftModels((a) => a.filter((x) => x !== m))}
                      title="移除"
                    >
                      <Icon name="x" size={10} />
                    </button>
                  </span>
                ))}
              </div>
            )}
            <div className="flex gap-1.5">
              <input
                className="field w-full px-3 py-1.5 text-xs"
                placeholder="模型 ID，如 deepseek-v4-flash"
                value={draftInput}
                onChange={(e) => setDraftInput(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") {
                    const v = draftInput.trim();
                    if (v && !draftModels.includes(v)) {
                      setDraftModels((a) => [...a, v]);
                      setDraftInput("");
                    }
                  }
                }}
              />
              <button
                type="button"
                className="btn-ghost flex shrink-0 items-center gap-1 px-3 py-1.5 text-xs"
                onClick={() => {
                  const v = draftInput.trim();
                  if (v && !draftModels.includes(v)) {
                    setDraftModels((a) => [...a, v]);
                    setDraftInput("");
                  }
                }}
              >
                <Icon name="plus" size={11} />
                新增
              </button>
            </div>
            <p className="text-[11px] text-fg/25">
              以上 {draftModels.length} 個將在儲存時一併建立；建立後可在此抓取清單、啟用/刪除。
            </p>
          </div>
        )}
      </div>

      {err && (
        <div className="rounded-lg border border-red-500/20 bg-red-500/[0.05] px-3 py-2 text-xs text-(--mac-red)">
          {err}
        </div>
      )}

      <div className="flex items-center justify-end gap-2.5 border-t border-fg/[0.06] pt-3">
        <button
          className="btn-ghost px-5 py-1.5 text-[13px]"
          onClick={props.onCancel}
        >
          取消
        </button>
        <button
          className="btn-primary px-6 py-1.5 text-[13px] disabled:opacity-40"
          disabled={save.isPending}
          onClick={() => save.mutate()}
        >
          {save.isPending ? "儲存中…" : "儲存"}
        </button>
      </div>
    </div>
  );
}
