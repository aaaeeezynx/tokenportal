/** 內建小工具的 API。
 *
 * 方法名加 `utilities` 前綴：`api` 是各模組 spread 合併的，通用名會覆蓋既有方法。
 */
import { invoke } from "@tauri-apps/api/core";
import type { UtilitiesState } from "../types/utilities";

export const utilitiesApi = {
  utilitiesState: (): Promise<UtilitiesState> => invoke("utilities_state"),
  /** 開啟／關閉一個小工具，回傳套用後的最新狀態。 */
  utilitiesSet: (key: string, enabled: boolean): Promise<UtilitiesState> =>
    invoke("utilities_set", { key, enabled }),
};
