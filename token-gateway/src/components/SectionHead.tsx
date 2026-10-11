/**
 * 全站唯一的區塊標題（原 settings/ 與 diagnostics/ 兩份合併，取 settings 版基準）。
 * - 間距：`mb-3`（12px）＋ `gap-3`（12px）——與全站「標題→內容」規範一致。
 * - caption：`text-fg/35`（原 diagnostics 版為 fg/30，已統一）。
 * 呼叫端不用改 import：兩個舊路徑各留一個 barrel 轉發。
 */
import type { ReactNode } from "react";
import { Icon, type IconName } from "./icons";

export function SectionHead(props: {
  icon: IconName;
  tile: string;
  title: string;
  caption?: string;
  /** 區塊右上角的動作區（如「重新整理」按鈕）。 */
  right?: ReactNode;
}) {
  return (
    <div className="mb-3 flex items-center gap-3">
      <div
        className="flex h-9 w-9 shrink-0 items-center justify-center rounded-[10px] text-fg"
        style={{ background: props.tile }}
      >
        <Icon name={props.icon} size={17} />
      </div>
      <div className="min-w-0 flex-1">
        <div className="text-[15px] font-semibold tracking-tight text-fg">
          {props.title}
        </div>
        {props.caption && (
          <div className="truncate text-[11px] text-fg/35">
            {props.caption}
          </div>
        )}
      </div>
      {props.right}
    </div>
  );
}
