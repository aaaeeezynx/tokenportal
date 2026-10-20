/** Claude Desktop 接管的 API。 */
import { invoke } from "@tauri-apps/api/core";
import type { DesktopStatus } from "../types/claudeDesktop";

/** 後端回傳 (狀態, 說明) 的二元組。 */
type WithMsg = [DesktopStatus, string];

export const claudeDesktopApi = {
  desktopStatus: (port: number): Promise<DesktopStatus> =>
    invoke("desktop_status", { port }),
  /** 建立缺少的別名 → 寫 profile → appliedId 指向本程式 */
  desktopApply: (port: number): Promise<WithMsg> =>
    invoke("desktop_apply", { port }),
  /** 刪 profile、移除 meta 條目、appliedId 指回其他 profile */
  desktopRestore: (port: number): Promise<WithMsg> =>
    invoke("desktop_restore", { port }),
};
