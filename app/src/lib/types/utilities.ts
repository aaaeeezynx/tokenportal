/** 內建小工具（對標 CC Switch 的 built-in utilities）的型別。 */

/** 後端 `utilities::UtilityTarget`。 */
export interface UtilityTarget {
  key: string;
  /** 會被修改的檔案完整路徑 */
  path: string;
  /** 該檔案目前是否存在 */
  exists: boolean;
}

/** 後端 `utilities::UtilitiesState`。 */
export interface UtilitiesState {
  skipOnboarding: boolean;
  hideAttribution: boolean;
  pluginIntegration: boolean;
  targets: UtilityTarget[];
}
