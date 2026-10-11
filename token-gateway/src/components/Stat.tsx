import { useLayoutEffect, useRef } from "react";

/**
 * 全站唯一的統計卡（原「用量總覽」與「診斷」兩套已合併，視覺基準＝用量頁）：
 * glass-float ＋ p-4、uppercase 標籤、20px 數值、hover 上浮、anim-rise 進場。
 *
 * - `tone`／`hint` 供診斷頁需要語義色與補充行時使用（用量頁不傳）。
 * - `delay` 是用量頁的交錯進場（每張 +50ms）。
 * - 數值**不做省略號截斷**：放不下時自動降字號（20→18→16px）——統計值以完整可讀優先。
 */
export function Stat(props: {
  label: string;
  value: number | string;
  tone?: "warn" | "ok" | "muted";
  hint?: string;
  /** 進場動畫延遲（ms）；不傳則不延遲。 */
  delay?: number;
}) {
  const color =
    props.tone === "warn"
      ? "var(--mac-red)"
      : props.tone === "ok"
        ? "var(--mac-green)"
        : props.tone === "muted"
          ? "var(--mac-label-2)"
          : undefined;
  const valRef = useRef<HTMLDivElement>(null);

  useLayoutEffect(() => {
    const el = valRef.current;
    if (!el) return;
    // 由大至小試到放得下為止；1.25rem=text-xl、1.125rem=text-lg、1rem=text-base
    const fit = () => {
      for (const s of ["1.25rem", "1.125rem", "1rem"]) {
        el.style.fontSize = s;
        if (el.scrollWidth <= el.clientWidth) break;
      }
    };
    fit();
    const ro =
      typeof ResizeObserver !== "undefined" ? new ResizeObserver(fit) : null;
    if (ro) ro.observe(el);
    return () => ro?.disconnect();
  }, [props.value]);

  return (
    <div
      className="glass-float float-hover anim-rise min-w-0 p-4"
      style={props.delay != null ? { animationDelay: `${props.delay}ms` } : undefined}
    >
      <div className="text-[11px] font-medium tracking-wider text-fg/35 uppercase">
        {props.label}
      </div>
      <div
        ref={valRef}
        className={`mt-1.5 overflow-hidden text-xl font-semibold whitespace-nowrap ${
          color ? "" : "text-fg/90"
        }`}
        style={color ? { color } : undefined}
      >
        {props.value}
      </div>
      {props.hint && (
        <div className="pt-1 text-[10px] leading-tight text-fg/25">
          {props.hint}
        </div>
      )}
    </div>
  );
}
