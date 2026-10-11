//! 由 `settings/SectionHead.tsx` 與 `diagnostics/SectionHead.tsx` 兩份合併為
//! `components/SectionHead.tsx`（同一視覺角色只留一套間距基準：gap-3 ＋ caption fg/35）。
//! 本檔僅保留對外匯出（barrel），維持既有 import 路徑。
export { SectionHead } from "../SectionHead";
