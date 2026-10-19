//! 從 **CC Switch v4.0.4** 同步的來源預設（157 筆）。
//!
//! ## 這份檔案是怎麼產生的
//!
//! 1. 下載 CC Switch 的四份預設資料檔（`claudeProviderPresets.ts`、
//!    `codexProviderPresets.ts`、`geminiProviderPresets.ts`、
//!    `opencodeProviderPresets.ts`），共 408 筆、去重後 389 筆。
//! 2. 依 `base_url` 正規化去重（同一個 URL 在不同工具下各有一份），
//!    只留 Token Portal 還沒有的 → 164 筆。
//! 3. 排除本機位址與範本佔位符 → 159 筆。
//! 4. **逐筆實測** `{base_url}/models`（不帶金鑰，24 執行緒）：
//!    只有 DNS 失敗／逾時／410 淘汰。結果 157 筆通過
//!    （89 筆 401、35 筆 404、28 筆 200、3 筆 400、1 筆 403、1 筆 405），
//!    淘汰 2 筆：`www.relaxycode.com`（DNS 不存在）、
//!    `YOUR_RESOURCE_NAME.openai.azure.com`（範本佔位符）。
//!
//! ## 協議與鑑權的對映
//!
//! | CC Switch `apiFormat` | 這裡的協議 | 鑑權 |
//! |---|---|---|
//! | （空，claude 來源） | `anthropic` | `bearer`（`ANTHROPIC_AUTH_TOKEN`）；`ANTHROPIC_API_KEY` 才是 `anthropic` |
//! | （空，codex 來源） | `openai-responses` | `bearer` |
//! | `openai_chat` | `openai-chat` | `bearer` |
//! | `openai_responses` | `openai-responses` | `bearer` |
//! | `gemini_native` | `gemini` | `goog-key` |
//!
//! ## 幾件刻意的事
//!
//! - **id 撞名會改**：CC Switch 的「DeepSeek」指向 `/anthropic`，與本專案既有的
//!   `deepseek`（`/v1`）是不同端點，所以這一份的 id 會加上網域後綴，不覆蓋舊的。
//! - **同名不同端點加協議後綴**：同一家的 Anthropic 端與 OpenAI 端都收錄時，
//!   顯示名會是「火山 Agent Plan（Anthropic）」「火山 Agent Plan（OpenAI）」。
//! - **不含模型清單**：與 `presets.rs` 的原則一致 —— 模型名變動快，
//!   一律由「取得模型清單」現場抓。
//! - 重跑方式：`.ccswitch-ref/` 的
//!   `parse_presets.py → compare.py → verify_urls.py → gen_table.py → assemble.py`
//!   （該目錄不進版控）。

/// (id, 名稱, base_url, 協議, 鑑權, 備註)
pub(crate) const CC_TABLE: [(&str, &str, &str, &str, &str, &str); 157] = [
    // ── 中國官方（CC Switch 同步）──
    ("agent-plan", "火山 Agent Plan（Anthropic）", "https://ark.cn-beijing.volces.com/api/plan", "anthropic", "bearer", ""),
    ("agent-plan-2", "火山 Agent Plan（OpenAI）", "https://ark.cn-beijing.volces.com/api/plan/v3", "openai-chat", "bearer", ""),
    ("ai-coding-plan", "千问AI平台 Coding Plan", "https://coding.dashscope.aliyuncs.com/apps/anthropic", "anthropic", "bearer", ""),
    ("ai-token-plan", "千问AI平台 Token Plan（OpenAI）", "https://token-plan.cn-beijing.maas.aliyuncs.com/apps/anthropic/v1", "openai-chat", "bearer", ""),
    ("ai-token-plan-2", "千问AI平台 Token Plan（Responses）", "https://token-plan.cn-beijing.maas.aliyuncs.com/compatible-mode/v1", "openai-responses", "bearer", ""),
    ("astron-coding-plan", "Astron Coding Plan", "https://maas-coding-api.cn-huabei-1.xf-yun.com/v1", "openai-responses", "bearer", ""),
    ("baidu-qianfan-coding-plan", "Baidu Qianfan Coding Plan（Anthropic）", "https://qianfan.baidubce.com/anthropic/coding", "anthropic", "bearer", ""),
    ("baidu-qianfan-coding-plan-2", "Baidu Qianfan Coding Plan（OpenAI）", "https://qianfan.baidubce.com/v2/coding", "openai-chat", "bearer", ""),
    ("baidu-qianfan-token-plan", "Baidu Qianfan Token Plan（Anthropic）", "https://qianfan.baidubce.com/anthropic/tokenplan/personal", "anthropic", "bearer", ""),
    ("baidu-qianfan-token-plan-2", "Baidu Qianfan Token Plan（OpenAI）", "https://qianfan.baidubce.com/v2/tokenplan/personal", "openai-chat", "bearer", ""),
    ("bailing", "BaiLing（OpenAI）", "https://api.ant-ling.com/v1", "openai-chat", "bearer", ""),
    ("bailing-2", "BaiLing（Anthropic）", "https://api.ant-ling.com/anthropic", "anthropic", "bearer", ""),
    ("byteplus", "BytePlus（Anthropic）", "https://ark.ap-southeast.bytepluses.com/api/coding", "anthropic", "bearer", ""),
    ("byteplus-2", "BytePlus（OpenAI）", "https://ark.ap-southeast.bytepluses.com/api/coding/v3", "openai-chat", "bearer", ""),
    ("coding-plan", "火山 Coding Plan（Anthropic）", "https://ark.cn-beijing.volces.com/api/coding", "anthropic", "bearer", ""),
    ("coding-plan-2", "火山 Coding Plan（OpenAI）", "https://ark.cn-beijing.volces.com/api/coding/v3", "openai-chat", "bearer", ""),
    ("dashscope-aliyuncs-apps-anthropic", "千问AI平台", "https://dashscope.aliyuncs.com/apps/anthropic", "anthropic", "bearer", ""),
    ("deepseek-api-deepseek", "DeepSeek", "https://api.deepseek.com/anthropic", "anthropic", "bearer", ""),
    ("kimi", "Kimi", "https://api.moonshot.cn/anthropic", "anthropic", "bearer", ""),
    ("kimi-for-coding", "Kimi For Coding", "https://api.kimi.com/coding/v1", "openai-chat", "bearer", ""),
    ("kimi-for-coding-global", "Kimi For Coding Global", "https://api.kimi.ai/coding/v1", "openai-chat", "bearer", ""),
    ("kimi-global", "Kimi Global（OpenAI）", "https://api.moonshot.ai/v1", "openai-chat", "bearer", ""),
    ("kimi-global-2", "Kimi Global（Anthropic）", "https://api.moonshot.ai/anthropic", "anthropic", "bearer", ""),
    ("longcat", "Longcat（Anthropic）", "https://api.longcat.chat/anthropic", "anthropic", "bearer", ""),
    ("longcat-2", "Longcat（OpenAI）", "https://api.longcat.chat/openai/v1", "openai-chat", "bearer", ""),
    ("meta-model-api", "Meta Model API", "https://api.meta.ai/v1", "openai-responses", "bearer", ""),
    ("minimax-api-minimax", "MiniMax（OpenAI）", "https://api.minimax.cn/v1", "openai-chat", "bearer", ""),
    ("minimax-2", "MiniMax（Anthropic）", "https://api.minimax.cn/anthropic", "anthropic", "bearer", ""),
    ("minimax-en", "MiniMax en（OpenAI）", "https://api.minimax.io/v1", "openai-chat", "bearer", ""),
    ("minimax-en-2", "MiniMax en（Anthropic）", "https://api.minimax.io/anthropic", "anthropic", "bearer", ""),
    ("qwencloud", "QwenCloud（Anthropic）", "https://dashscope-intl.aliyuncs.com/apps/anthropic", "anthropic", "bearer", ""),
    ("qwencloud-2", "QwenCloud（OpenAI）", "https://dashscope-intl.aliyuncs.com/compatible-mode/v1", "openai-chat", "bearer", ""),
    ("qwencloud-for-coding", "QwenCloud For Coding（OpenAI）", "https://coding-intl.dashscope.aliyuncs.com/v1", "openai-chat", "bearer", ""),
    ("qwencloud-for-coding-2", "QwenCloud For Coding（OpenAI）", "https://coding-intl.dashscope.aliyuncs.com/apps/anthropic/v1", "openai-chat", "bearer", ""),
    ("qwencloud-token-plan", "QwenCloud Token Plan（OpenAI）", "https://token-plan.ap-southeast-1.maas.aliyuncs.com/apps/anthropic/v1", "openai-chat", "bearer", ""),
    ("qwencloud-token-plan-2", "QwenCloud Token Plan（Responses）", "https://token-plan.ap-southeast-1.maas.aliyuncs.com/compatible-mode/v1", "openai-responses", "bearer", ""),
    ("stepfun-api-stepfun", "StepFun", "https://api.stepfun.com/step_plan/v1", "openai-chat", "bearer", ""),
    ("stepfun-api-en", "StepFun API en", "https://api.stepfun.ai/v1", "openai-responses", "bearer", ""),
    ("stepfun-en", "StepFun en", "https://api.stepfun.ai/step_plan/v1", "openai-chat", "bearer", ""),
    ("streamlake", "StreamLake（OpenAI）", "https://wanqing.streamlakeapi.com/api/gateway/v1/endpoints", "openai-chat", "bearer", ""),
    ("streamlake-2", "StreamLake（Anthropic）", "https://wanqing.streamlakeapi.com/api/gateway/v1/endpoints/kat-coder-pro-v2.5/claude-code-proxy", "anthropic", "bearer", ""),
    ("streamlake-en", "StreamLake en（OpenAI）", "https://vanchin.streamlake.ai/api/gateway/v1/endpoints", "openai-chat", "bearer", ""),
    ("streamlake-en-2", "StreamLake en（Anthropic）", "https://vanchin.streamlake.ai/api/gateway/v1/endpoints/kat-coder-pro-v2.5/claude-code-proxy", "anthropic", "bearer", ""),
    ("tencent-hunyuan", "Tencent Hunyuan", "https://tokenhub.tencentmaas.com/v1", "openai-responses", "bearer", ""),
    ("tencent-token-plan", "Tencent Token Plan（Anthropic）", "https://api.lkeap.cloud.tencent.com/plan/anthropic", "anthropic", "bearer", ""),
    ("tencent-token-plan-2", "Tencent Token Plan（OpenAI）", "https://api.lkeap.cloud.tencent.com/plan/v3", "openai-chat", "bearer", ""),
    ("tencent-token-plan-3", "Tencent Token Plan (Intl)（Anthropic）", "https://tokenhub-intl.tencentcloudmaas.com/plan/anthropic", "anthropic", "bearer", ""),
    ("tencent-token-plan-4", "Tencent Token Plan (Intl)（OpenAI）", "https://tokenhub-intl.tencentcloudmaas.com/plan/v3", "openai-chat", "bearer", ""),
    ("tencent-token-plan-enterprise-pro", "Tencent Token Plan Enterprise Pro（Anthropic）", "https://tokenhub.tencentmaas.com/plan/anthropic", "anthropic", "bearer", ""),
    ("tencent-token-plan-enterprise-pro-2", "Tencent Token Plan Enterprise Pro（OpenAI）", "https://tokenhub.tencentmaas.com/plan/v3", "openai-chat", "bearer", ""),
    ("volcengine-doubao", "Volcengine Doubao（Anthropic）", "https://ark.cn-beijing.volces.com/api/compatible", "anthropic", "bearer", ""),
    ("volcengine-doubao-2", "Volcengine Doubao（OpenAI）", "https://ark.cn-beijing.volces.com/api/v3", "openai-chat", "bearer", ""),
    ("xiaomi-mimo", "Xiaomi MiMo（OpenAI）", "https://api.xiaomimimo.com/v1", "openai-chat", "bearer", ""),
    ("xiaomi-mimo-2", "Xiaomi MiMo（Anthropic）", "https://api.xiaomimimo.com/anthropic", "anthropic", "bearer", ""),
    ("xiaomi-mimo-token-plan", "Xiaomi MiMo Token Plan (China)（OpenAI）", "https://token-plan-cn.xiaomimimo.com/v1", "openai-chat", "bearer", ""),
    ("xiaomi-mimo-token-plan-2", "Xiaomi MiMo Token Plan (China)（Anthropic）", "https://token-plan-cn.xiaomimimo.com/anthropic", "anthropic", "bearer", ""),
    ("zhipu-glm", "Zhipu GLM（Responses）", "https://open.bigmodel.cn/api/v1", "openai-responses", "bearer", ""),
    ("zhipu-glm-2", "Zhipu GLM（Anthropic）", "https://open.bigmodel.cn/api/anthropic", "anthropic", "bearer", ""),
    ("zhipu-glm-3", "Zhipu GLM（OpenAI）", "https://open.bigmodel.cn/api/coding/paas/v4", "openai-chat", "bearer", ""),
    ("zhipu-glm-en", "Zhipu GLM en（Responses）", "https://api.z.ai/api/v1", "openai-responses", "bearer", ""),
    ("zhipu-glm-en-2", "Zhipu GLM en（Anthropic）", "https://api.z.ai/api/anthropic", "anthropic", "bearer", ""),
    ("zhipu-glm-en-3", "Zhipu GLM en（OpenAI）", "https://api.z.ai/api/coding/paas/v4", "openai-chat", "bearer", ""),

    // ── 聚合／轉售（CC Switch 同步）──
    ("88api", "88API", "https://api.88api.ai/v1", "openai-chat", "bearer", ""),
    ("9527code", "9527CODE", "https://9527.codes/v1", "openai-chat", "bearer", ""),
    ("a6api", "A6API", "https://api.a6api.com/v1", "openai-chat", "bearer", ""),
    ("aicodewith", "AICodeWith（OpenAI）", "https://api.aicodewith.ai/v1", "openai-chat", "bearer", ""),
    ("aicodewith-2", "AICodeWith（Gemini）", "https://api.aicodewith.ai/gemini_cli", "gemini", "goog-key", ""),
    ("aicoming", "AIComing", "https://api.aicoming.top/v1", "openai-chat", "bearer", ""),
    ("alltokens", "AllTokens", "https://alltokens.cc/v1", "openai-chat", "bearer", ""),
    ("alltokens-si", "AllTokens SI", "https://alltokens.si/v1", "openai-chat", "bearer", ""),
    ("amux", "Amux", "https://api.amux.ai/v1", "openai-chat", "bearer", ""),
    ("api-moark-v1", "模力方舟（OpenAI）", "https://api.moark.com/v1", "openai-chat", "bearer", ""),
    ("atlascloud", "AtlasCloud", "https://api.atlascloud.ai/v1", "openai-chat", "bearer", ""),
    ("baseten", "Baseten", "https://inference.baseten.co", "openai-chat", "bearer", ""),
    ("botcf", "BotCF", "https://botcf.com/v1", "openai-chat", "bearer", ""),
    ("ccsub", "CCSub", "https://www.ccsub.net/v1", "openai-chat", "bearer", ""),
    ("cherryin", "CherryIN", "https://open.cherryin.net/v1", "openai-chat", "bearer", ""),
    ("claudeapi", "ClaudeAPI", "https://gw.apito.ai", "anthropic", "bearer", ""),
    ("code0", "Code0", "https://code0.ai/v1", "openai-chat", "bearer", ""),
    ("compshare", "Compshare", "https://api.modelverse.cn/v1", "openai-responses", "bearer", ""),
    ("compshare-coding-plan", "Compshare Coding Plan", "https://cp.compshare.cn/v1", "openai-responses", "bearer", ""),
    ("deepinfra-api-deepinfra", "DeepInfra（Responses）", "https://api.deepinfra.com/v1", "openai-responses", "bearer", ""),
    ("deepinfra-2", "DeepInfra（Anthropic）", "https://api.deepinfra.com/anthropic", "anthropic", "bearer", ""),
    ("dmxapi", "DMXAPI", "https://www.dmxapi.cn/v1", "openai-chat", "bearer", ""),
    ("everyapi", "EveryAPI", "https://api.everyapi.ai/v1", "openai-chat", "bearer", ""),
    ("fennoai", "FennoAI", "https://api.fenno.ai/v1", "openai-chat", "bearer", ""),
    ("fluxa-token-plan", "FluxA Token Plan（Anthropic）", "https://api.baiduqianfan.ai/anthropic/tokenplan/team", "anthropic", "bearer", ""),
    ("fluxa-token-plan-2", "FluxA Token Plan（OpenAI）", "https://api.baiduqianfan.ai/v2/tokenplan/team", "openai-chat", "bearer", ""),
    ("grooroute-pro", "GrooRoute Pro", "https://grooroute.pro/v1", "openai-chat", "bearer", ""),
    ("hugging-face", "Hugging Face", "https://router.huggingface.co/v1", "openai-chat", "bearer", ""),
    ("ikuncode", "IKunCode", "https://api.ikuncode.ai/v1", "openai-chat", "bearer", ""),
    ("jiekou-ai", "JieKou AI（Anthropic）", "https://api.jiekou.ai/anthropic", "anthropic", "bearer", ""),
    ("jiekou-ai-2", "JieKou AI（OpenAI）", "https://api.jiekou.ai/openai/v1", "openai-chat", "bearer", ""),
    ("moark-anthropic", "模力方舟（Anthropic）", "https://moark.com/anthropic", "anthropic", "bearer", ""),
    ("moark-v1", "模力方舟（Responses）", "https://moark.com/v1", "openai-responses", "bearer", ""),
    ("modelflare", "Modelflare", "https://modelflare.dev/v1", "openai-chat", "bearer", ""),
    ("nebius-token-factory", "Nebius Token Factory", "https://api.tokenfactory.nebius.com", "openai-chat", "bearer", ""),
    ("nekocode", "NekoCode", "https://nekoapi.ai/v1", "openai-chat", "bearer", ""),
    ("novita-ai", "Novita AI（OpenAI）", "https://api.novita.ai/openai", "openai-chat", "bearer", ""),
    ("novita-ai-2", "Novita AI（Anthropic）", "https://api.novita.ai/anthropic", "anthropic", "bearer", ""),
    ("novita-ai-3", "Novita AI（OpenAI）", "https://api.novita.ai/openai/v1", "openai-chat", "bearer", ""),
    ("opencode-zen", "OpenCode Zen", "https://opencode.ai/zen/v1", "openai-responses", "bearer", ""),
    ("openmodel", "OpenModel", "https://api.openmodel.ai/v1", "openai-chat", "bearer", ""),
    ("pipellm", "PIPELLM", "https://cc-api.pipellm.ai", "openai-chat", "bearer", ""),
    ("ppio-api-ppio", "PPIO（Anthropic）", "https://api.ppio.com/anthropic", "anthropic", "bearer", ""),
    ("ppio-2", "PPIO（OpenAI）", "https://api.ppio.com/openai/v1", "openai-chat", "bearer", ""),
    ("qiniu", "Qiniu（OpenAI）", "https://api.qnaigc.com/v1", "openai-chat", "bearer", ""),
    ("qiniu-2", "Qiniu（Responses）", "https://api.qnaigc.com/bypass/openai/v1", "openai-responses", "bearer", ""),
    ("qiniu-3", "Qiniu（Gemini）", "https://api.qnaigc.com/bypass/vertex", "gemini", "goog-key", ""),
    ("runapi", "RunAPI", "https://runapi.host", "openai-chat", "bearer", ""),
    ("runtoken", "RunToken", "https://www.runtoken.ai/v1", "openai-chat", "bearer", ""),
    ("shengsuanyun", "Shengsuanyun", "https://router.shengsuanyun.com/api/v1", "openai-chat", "bearer", ""),
    ("siliconflow-en", "SiliconFlow en", "https://api.siliconflow.com/v1", "openai-chat", "bearer", ""),
    ("soleapi", "SoleAPI", "https://soleapi.com/v1", "openai-chat", "bearer", ""),
    ("soruxgpt", "SoruxGPT", "https://ai.soruxgpt.com/v1", "openai-chat", "bearer", ""),
    ("soruxgpt-coding-plan", "SoruxGPT Coding Plan（Anthropic）", "https://app.soruxgpt.com/api/cc", "anthropic", "bearer", ""),
    ("soruxgpt-coding-plan-2", "SoruxGPT Coding Plan（Responses）", "https://app.soruxgpt.com/api/codex", "openai-responses", "bearer", ""),
    ("soshow", "Soshow", "https://maas.so-show.com/v1", "openai-chat", "bearer", ""),
    ("subrouter", "SubRouter", "https://subrouter.ai/v1", "openai-chat", "bearer", ""),
    ("teamorouter", "TeamoRouter", "https://api.teamorouter.cn/v1", "openai-chat", "bearer", ""),
    ("therouter", "TheRouter", "https://api.therouter.ai/v1", "openai-chat", "bearer", ""),
    ("tokenfactory", "TokenFactory（OpenAI）", "https://api.tokenfactory.cn/v1", "openai-chat", "bearer", ""),
    ("tokenfactory-2", "TokenFactory（Anthropic）", "https://api.tokenfactory.cn/anthropic", "anthropic", "bearer", ""),
    ("tokenflux", "TokenFlux", "https://tokenflux.dev/v1", "openai-chat", "bearer", ""),
    ("tu-zi", "Tu-zi", "https://api.tu-zi.com/v1", "openai-chat", "bearer", ""),
    ("vercel-ai-gateway", "Vercel AI Gateway（Anthropic）", "https://ai-gateway.vercel.sh/claude-code", "anthropic", "bearer", ""),
    ("vercel-ai-gateway-2", "Vercel AI Gateway（Responses）", "https://ai-gateway.vercel.sh/codex/v1", "openai-responses", "bearer", ""),
    ("xycai", "XycAi", "https://apicdn.xycai.us/v1", "openai-chat", "bearer", ""),
    ("yylx", "YYLX", "https://app.yylx.io/v1", "openai-chat", "bearer", ""),
    ("zetaapi", "ZetaAPI", "https://api.zetaapi.ai/v1", "openai-chat", "bearer", ""),

    // ── 第三方（CC Switch 同步）──
    ("aicodemirror", "AICodeMirror（OpenAI）", "https://api.aicodemirror.ai/api/claudecode", "openai-chat", "bearer", ""),
    ("aicodemirror-3", "AICodeMirror（Gemini）", "https://api.aicodemirror.ai/api/gemini", "gemini", "goog-key", ""),
    ("aicoding", "AICoding", "https://api.aicoding.inc", "openai-chat", "bearer", ""),
    ("aigocode", "AIGoCode", "https://api.aigocode.dev", "openai-chat", "bearer", ""),
    ("apikey-fun", "APIKEY.FUN", "https://api.apikey.fan/v1", "openai-chat", "bearer", ""),
    ("apinebula", "APINebula", "https://apinebula.ai/v1", "openai-chat", "bearer", ""),
    ("claudecn", "ClaudeCN", "https://claudecn.top", "openai-chat", "bearer", ""),
    ("codex", "Codex", "https://chatgpt.com/backend-api/codex", "openai-responses", "bearer", ""),
    ("command-code", "Command Code", "https://api.commandcode.ai/provider", "openai-chat", "bearer", ""),
    ("crazyrouter", "CrazyRouter", "https://cn.crazyrouter.com", "openai-chat", "bearer", ""),
    ("cubence", "Cubence", "https://api.cubence.com/v1", "openai-chat", "bearer", ""),
    ("e-flowcode", "E-FlowCode", "https://e-flowcode.cc/v1", "openai-chat", "bearer", ""),
    ("github-copilot", "GitHub Copilot", "https://api.githubcopilot.com", "openai-chat", "bearer", ""),
    ("micu", "Micu", "https://www.micuapi.ai/v1", "openai-chat", "bearer", ""),
    ("opencode-go-opencode", "OpenCode Go", "https://opencode.ai/zen/go/v1", "openai-chat", "bearer", ""),
    ("packycode", "PackyCode", "https://www.packyapi.ai/v1", "openai-chat", "bearer", ""),
    ("patewayai", "PatewayAI", "https://api.pateway.ai/v1", "openai-responses", "bearer", ""),
    ("rightcode", "RightCode（Anthropic）", "https://www.rightapi.ai/claude", "anthropic", "bearer", ""),
    ("rightcode-2", "RightCode（OpenAI）", "https://www.rightapi.ai/codex/v1", "openai-chat", "bearer", ""),
    ("sssaicode", "SSSAiCode", "https://node-hk.sssaicodeapi.com/api/v1", "openai-chat", "bearer", ""),
    ("sudocode-chat", "SudoCode.chat", "https://api.sudocode.chat/v1", "openai-chat", "bearer", ""),
    ("sudocode-us", "SudoCode.us", "https://sudocode.us/v1", "openai-chat", "bearer", ""),

    // ── 雲廠商（CC Switch 同步）──
    ("cloudflare-ai-gateway", "Cloudflare AI Gateway", "https://gateway.ai.cloudflare.com/v1/YOUR_ACCOUNT_ID/default/openai", "openai-responses", "bearer", ""),
    ("digitalocean", "DigitalOcean", "https://inference.do-ai.run/v1", "openai-responses", "bearer", ""),
    ("ollama-cloud", "Ollama Cloud", "https://ollama.com/v1", "openai-responses", "bearer", ""),
    ("sambanova", "SambaNova", "https://api.sambanova.ai/v1", "openai-responses", "bearer", ""),

    // ── 其他（CC Switch 同步）──
    ("aicodemirror-2", "AICodeMirror（Responses）", "https://api.aicodemirror.ai/api/codex/backend-api/codex", "openai-responses", "bearer", ""),
];
