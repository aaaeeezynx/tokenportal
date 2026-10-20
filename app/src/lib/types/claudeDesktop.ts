/** Claude Desktop 接管的型別（對應後端 `claude_desktop.rs`）。 */

export interface DesktopAlias {
  name: string;
  /** 實際會送上游的模型名（null = 還沒建立對映） */
  actual: string | null;
  /** 是哪個來源登記的 */
  provider: string | null;
}

export interface DesktopStatus {
  installed: boolean;
  /** 我們那份 profile 是否為目前生效的 */
  configured: boolean;
  profileId: string;
  configLibrary: string;
  profilePath: string;
  baseUrl: string;
  keyName: string | null;
  aliases: DesktopAlias[];
  note: string;
}
