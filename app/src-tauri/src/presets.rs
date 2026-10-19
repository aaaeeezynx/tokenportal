//! 來源預設集（對標 CC Switch 的「90+ provider presets」）。
//!
//! ## 這份表是怎麼來的（不要憑記憶新增）
//!
//! 每一筆的 `base_url` 都在 2026-09-30 **實測過**：對 `{base_url}/models` 送一次
//! 不帶金鑰的 GET，任何 HTTP 狀態（200／401／403／404）都代表主機與路徑存在，
//! 只有 DNS 失敗／連線逾時／410 才淘汰。當時淘汰了 `api.sambanova.ai`、
//! `llm.chutes.ai`、`api.kluster.ai`、`api.lambdalabs.com`（連不上）、
//! `ark.cn-beijing.volces.com`（逾時）與 `api.lingyiwanwu.com`（410 已下線）。
//!
//! 兩個本機服務（Ollama／LM Studio）在這台機器上沒有安裝，因此**沒有實測**，
//! 備註欄如實寫出來；它們的位址是各自的預設埠。
//!
//! ## 為什麼不含模型清單
//!
//! CC Switch 的預設會順便填幾個模型名。我們刻意不填：模型名變動快，猜錯比不填更糟；
//! 新建來源表單有「取得模型清單」會直接對上游抓 `/models`（`catalog_fetch`）。
//! 也就是：預設只負責**連線資訊**，模型一律以現場抓到的為準。
//!
//! ## 2026-10-19：從 CC Switch v4.0.4 同步（38 → 195 筆）
//!
//! 本檔的 38 筆是人工挑選、逐筆實測過的（2026-09-30）；另外 157 筆放在
//! [`crate::presets_cc`]，是從 CC Switch 的四份預設資料檔同步後**重新實測**
//! 才收進來的（產生方式與淘汰清單見該檔開頭）。兩張表合起來 195 筆，
//! 但**來源與維護方式不同，所以刻意分成兩個檔案**：
//!
//! - 本檔：人工維護，改動時要重跑實測。
//! - `presets_cc.rs`：機器產生，重新同步時整檔換掉。
//!
//! 命名衝突一律「加後綴」而不是「覆蓋」——例如 CC Switch 的 `DeepSeek`
//! 指向 `/anthropic`，與本檔的 `deepseek`（`/v1`）是不同端點，兩者都留。

use serde::Serialize;

/// 一個來源預設。
#[derive(Debug, Clone, Serialize)]
pub struct Preset {
    pub id: &'static str,
    pub name: &'static str,
    pub base_url: &'static str,
    /// `openai-chat`／`openai-responses`／`mixed`／`anthropic`／`gemini`
    pub api_format: &'static str,
    /// `bearer`／`goog-key`／`anthropic`
    pub auth_scheme: &'static str,
    /// 一律 `universal`：`app_type` 在本 App 只用於顯示與分類，
    /// 不影響路由（路由是照「哪個來源登記了這個模型」找的）。
    pub app_type: &'static str,
    /// 逐協議路徑前綴的 JSON（空 = 沒有）。見 `models/prefixes.rs`。
    pub protocol_prefixes: &'static str,
    pub note: &'static str,
}

/// 需要「逐協議路徑前綴」的預設（其餘一律沒有）。
///
/// 只有把協議做在**路徑前綴**裡的上游才需要它。實測 OpenCode Go 的文件端點是
/// 同一個 base 底下 `/openai/v1/…`、`/anthropic/v1/…`、`/google/v1beta/…` ——
/// 用單一 base 加字尾描述不了，所以連前綴一起給，**一個來源就能服務全部四種協議**。
fn preset_prefixes(id: &str) -> &'static str {
    match id {
        "opencode-go" => {
            r#"{"openai-chat":"/openai/v1","openai-responses":"/openai/v1","anthropic":"/anthropic/v1","gemini":"/google/v1beta"}"#
        }
        _ => "",
    }
}

/// 一列的形狀：(id, 名稱, base_url, 協議, 鑑權, 備註)。
pub(crate) type Row = (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
);

/// (id, 名稱, base_url, 協議, 鑑權, 備註)
const TABLE: [Row; 38] = [
    // ── 官方 ──
    ("openai", "OpenAI 官方", "https://api.openai.com/v1", "openai-chat", "bearer", ""),
    (
        "openai-responses",
        "OpenAI 官方（Responses）",
        "https://api.openai.com/v1",
        "openai-responses",
        "bearer",
        "只走 /responses 端點",
    ),
    ("anthropic", "Anthropic 官方", "https://api.anthropic.com", "anthropic", "anthropic", ""),
    ("gemini", "Google Gemini", "https://generativelanguage.googleapis.com/v1beta", "gemini", "goog-key", "原生 Gemini 協定，不做翻譯"),
    ("xai", "xAI Grok", "https://api.x.ai/v1", "openai-chat", "bearer", "部分模型只在 /responses，網關會自動換手並記住"),
    // ── 聚合／推理服務 ──
    ("deepseek", "DeepSeek", "https://api.deepseek.com/v1", "openai-chat", "bearer", ""),
    ("openrouter", "OpenRouter", "https://openrouter.ai/api/v1", "openai-chat", "bearer", ""),
    ("siliconflow", "硅基流動 SiliconFlow", "https://api.siliconflow.cn/v1", "openai-chat", "bearer", ""),
    ("moonshot", "Moonshot / Kimi", "https://api.moonshot.cn/v1", "openai-chat", "bearer", ""),
    ("zhipu", "智譜 GLM", "https://open.bigmodel.cn/api/paas/v4", "openai-chat", "bearer", ""),
    ("zai", "Z.ai（GLM 國際站）", "https://api.z.ai/api/paas/v4", "openai-chat", "bearer", ""),
    ("minimax", "MiniMax", "https://api.minimax.chat/v1", "openai-chat", "bearer", ""),
    ("minimaxi", "MiniMax（國際站）", "https://api.minimaxi.com/v1", "openai-chat", "bearer", ""),
    ("groq", "Groq", "https://api.groq.com/openai/v1", "openai-chat", "bearer", ""),
    ("mistral", "Mistral", "https://api.mistral.ai/v1", "openai-chat", "bearer", ""),
    ("together", "Together AI", "https://api.together.xyz/v1", "openai-chat", "bearer", ""),
    ("fireworks", "Fireworks AI", "https://api.fireworks.ai/inference/v1", "openai-chat", "bearer", ""),
    ("cerebras", "Cerebras", "https://api.cerebras.ai/v1", "openai-chat", "bearer", ""),
    ("deepinfra", "DeepInfra", "https://api.deepinfra.com/v1/openai", "openai-chat", "bearer", ""),
    ("hyperbolic", "Hyperbolic", "https://api.hyperbolic.xyz/v1", "openai-chat", "bearer", ""),
    ("nebius", "Nebius AI Studio", "https://api.studio.nebius.com/v1", "openai-chat", "bearer", ""),
    ("novita", "Novita AI", "https://api.novita.ai/v3/openai", "openai-chat", "bearer", ""),
    ("nvidia-nim", "NVIDIA NIM", "https://integrate.api.nvidia.com/v1", "openai-chat", "bearer", ""),
    (
        "perplexity",
        "Perplexity",
        "https://api.perplexity.ai",
        "openai-chat",
        "bearer",
        "上游沒有 /models 端點，模型要手動填",
    ),
    // ── 中國大陸雲 ──
    ("qianfan", "百度千帆", "https://qianfan.baidubce.com/v2", "openai-chat", "bearer", ""),
    ("dashscope", "阿里雲百鍊（兼容模式）", "https://dashscope.aliyuncs.com/compatible-mode/v1", "openai-chat", "bearer", ""),
    ("hunyuan", "騰訊混元", "https://api.hunyuan.cloud.tencent.com/v1", "openai-chat", "bearer", ""),
    ("stepfun", "階躍星辰", "https://api.stepfun.com/v1", "openai-chat", "bearer", ""),
    ("baichuan", "百川智能", "https://api.baichuan-ai.com/v1", "openai-chat", "bearer", ""),
    ("modelscope", "ModelScope 創空間", "https://api-inference.modelscope.cn/v1", "openai-chat", "bearer", ""),
    ("ppio", "PPIO 派歐雲", "https://api.ppinfra.com/v3/openai", "openai-chat", "bearer", ""),
    // ── 中轉／閘道 ──
    ("requesty", "Requesty", "https://router.requesty.ai/v1", "openai-chat", "bearer", ""),
    ("aihubmix", "AiHubMix", "https://aihubmix.com/v1", "openai-chat", "bearer", ""),
    ("ai302", "302.AI", "https://api.302.ai/v1", "openai-chat", "bearer", ""),
    ("gptsapi", "GPTSAPI", "https://api.gptsapi.net/v1", "openai-chat", "bearer", ""),
    ("opencode-go", "OpenCode Go（Zen）", "https://opencode.ai/inference/go", "auto", "bearer", "依官方文件：同一個 base 底下 /openai/v1、/anthropic/v1、/google/v1beta 各是一種協議；已填好逐協議前綴，四種協議一個來源搞定"),
    // ── 本機（未實測：此機未安裝）──
    ("ollama", "Ollama（本機）", "http://localhost:11434/v1", "openai-chat", "bearer", "本機服務，需先啟動 Ollama；此機未安裝故未實測"),
    ("lmstudio", "LM Studio（本機）", "http://localhost:1234/v1", "openai-chat", "bearer", "本機服務，需先在 LM Studio 開啟伺服器；未實測"),
];

/// 全部列：本檔手動維護的 38 筆 ＋ 從 CC Switch 同步的 157 筆。
///
/// 分成兩個檔案不是為了美觀：`presets.rs` 的每一筆都是人工挑選並在
/// 2026-09-30 實測過的，`presets_cc.rs` 是 2026-10-19 從上游同步後批次實測的，
/// 來源與維護方式不同，混在一起會讓「哪一筆是誰加的」說不清楚。
fn rows() -> impl Iterator<Item = &'static Row> {
    TABLE.iter().chain(crate::presets_cc::CC_TABLE.iter())
}

/// 全部預設（前端「從預設開始」清單）。
#[tauri::command]
pub fn presets_list() -> Vec<Preset> {
    rows()
        .map(|(id, name, base_url, api_format, auth_scheme, note)| Preset {
            id,
            name,
            base_url,
            api_format,
            auth_scheme,
            app_type: "universal",
            protocol_prefixes: preset_prefixes(id),
            note,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// id 是前端的鍵，重複會讓「選了卻填錯」。
    ///
    /// 兩張表合起來一起驗：`presets_cc.rs` 是產生的，最容易撞名。
    #[test]
    fn preset_ids_are_unique() {
        let mut ids: Vec<&str> = rows().map(|p| p.0).collect();
        let n = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), n, "預設 id 有重複");
    }

    /// 每一個預設都必須是「填進去就能用」的完整連線資訊。
    #[test]
    fn presets_are_well_formed() {
        // `auto` 是 2026-10-10 起的預設（逐模型偵測協議）
        const FORMATS: [&str; 6] = [
            "openai-chat",
            "openai-responses",
            "mixed",
            "anthropic",
            "gemini",
            "auto",
        ];
        const SCHEMES: [&str; 3] = ["bearer", "goog-key", "anthropic"];
        let all: Vec<&Row> = rows().collect();
        for (id, name, base_url, api_format, auth_scheme, _note) in all.iter().copied() {
            assert!(!id.trim().is_empty(), "id 不可為空");
            assert!(!name.trim().is_empty(), "{id}：名稱不可為空");
            assert!(
                base_url.starts_with("https://") || base_url.starts_with("http://localhost"),
                "{id}：base_url 必須是 https（或 localhost 的本機服務）：{base_url}"
            );
            assert!(!base_url.ends_with('/'), "{id}：base_url 結尾不該有斜線");
            assert!(FORMATS.contains(api_format), "{id}：未知協議 {api_format}");
            assert!(SCHEMES.contains(auth_scheme), "{id}：未知鑑權 {auth_scheme}");
            assert_eq!(
                all.iter().filter(|p| p.0 == *id).count(),
                1,
                "{id} 重複"
            );
        }
    }

    /// 命令輸出的形狀（前端靠這個 id 對映）。
    #[test]
    fn list_is_serializable_and_complete() {
        let list = presets_list();
        assert_eq!(list.len(), rows().count());
        assert_eq!(list.len(), TABLE.len() + crate::presets_cc::CC_TABLE.len());
        assert!(list.iter().all(|p| p.app_type == "universal"));
        let json = serde_json::to_string(&list).unwrap();
        assert!(json.contains("\"openai\""));
        assert!(json.contains("\"openai-chat\""));
        // 至少要有使用者實際在用的那一個（回歸：oc-go 是本案的起點）
        // 2026-10-11：改用官方文件記載的 base（同一個 base 底下四種協議各有前綴）
        assert!(list.iter().any(|p| p.base_url.contains("opencode.ai/inference/go")));
        // 只有它需要逐協議前綴
        assert!(list
            .iter()
            .any(|p| p.id == "opencode-go" && !p.protocol_prefixes.is_empty()));
        assert!(list
            .iter()
            .filter(|p| p.id != "opencode-go")
            .all(|p| p.protocol_prefixes.is_empty()));
    }
}
