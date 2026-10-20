//! 工具安裝、全部升級、重複安裝診斷（對標 CC Switch 的 Apps 頁）。
//!
//! 既有的 `update.rs` 只做「查版本 ＋ 更新單一工具」。這裡補三件它沒有的：
//!
//! | 功能 | 為什麼需要 |
//! |---|---|
//! | **一鍵安裝** | 沒裝的工具目前只能叫使用者自己去終端機打 `npm i -g …` |
//! | **全部升級** | 四個工具逐一按很煩；這裡只升級「已安裝且確實落後」的 |
//! | **重複安裝診斷** | 同一個工具有兩份時，**跑的那份不一定是剛更新的那份** —— 這是最容易誤判「更新沒生效」的原因 |
//!
//! ## 重複的判定：以「目錄」為單位，不是以檔案
//!
//! Windows 上 npm 會在**同一個目錄**同時產生 `claude.cmd` 與 `claude.ps1`
//! （shim 與 PowerShell 版本），那是同一份安裝的兩個入口，**不是重複安裝**。
//! 所以判定單位是「檔案所在目錄」：不同目錄各有一份才算重複。
//!
//! ## 為什麼用 `npm i -g` 而不是各自的自我更新
//!
//! `claude update` 之類的自我更新會把檔案寫到自己所在的位置，若有多份副本，
//! 更新的是「哪一份」取決於當下解析到的路徑 —— 難以預期。統一走 npm 全域安裝，
//! 目標明確（`npm_package(app)` 指定的那個包），也與既有的 `tool_update_run` 一致。

use std::path::PathBuf;

use serde::Serialize;

use super::probe::probe_tool_version;
use super::update::{npm_latest, tool_update_argv};
use super::{npm_package, run_with_timeout, which_bin};

/// 一份副本。
#[derive(Debug, Clone, Serialize)]
pub struct ToolCopy {
    /// 檔案完整路徑
    pub path: String,
    /// 這份是不是「PATH 上第一個」＝實際會被執行的
    pub active: bool,
}

/// 重複安裝診斷結果（一個工具一筆）。
#[derive(Debug, Clone, Serialize)]
pub struct DuplicateReport {
    pub app: String,
    pub display: String,
    /// 不同目錄的副本（`active` 為真的排第一）
    pub copies: Vec<ToolCopy>,
    /// 是否找到兩份以上（不同目錄）
    pub duplicated: bool,
}

/// 全部升級的單筆結果。
#[derive(Debug, Clone, Serialize)]
pub struct UpdateOutcome {
    pub app: String,
    pub display: String,
    /// 這次有沒有真的跑更新（已是最新或未安裝 → false）
    pub ran: bool,
    pub ok: bool,
    pub from: Option<String>,
    pub to: Option<String>,
    pub message: String,
}

/// 在給定的目錄清單裡找出該工具的所有入口檔。
///
/// **只回存在的檔案**，並保留目錄順序（與 PATH 的優先序一致）。
/// 抽成「吃目錄清單」的形狀是為了可測試 —— 正式路徑傳 PATH，
/// 測試傳一個臨時目錄。
///
/// ## Windows 的副檔名優先序（實測踩過）
///
/// npm 在 `%APPDATA%\npm\` 會同時產生 `claude`（給 Git Bash 的 shell script）、
/// `claude.cmd`（cmd／PowerShell 真正執行的那個）與 `claude.ps1`。
/// 舊寫法先找無副檔名的，於是診斷回報 `...\npm\claude` —— 那在 Windows 上
/// **根本不能執行**，使用者照著比對會一頭霧水。所以 Windows 先找可執行副檔名，
/// 無副檔名的排最後。
pub(crate) fn find_all_bins_in(name: &str, dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for d in dirs {
        let mut hits: Vec<PathBuf> = Vec::new();
        #[cfg(windows)]
        for ext in ["exe", "cmd", "bat", "ps1"] {
            let p = d.join(format!("{name}.{ext}"));
            if p.is_file() {
                hits.push(p);
            }
        }
        // 無副檔名的（Unix 風格／npm 的 Git Bash shim）排在 Windows 副檔名之後
        let base = d.join(name);
        if base.is_file() {
            hits.push(base);
        }
        // 同一個目錄只保留第一個（cmd 與 ps1 是同一份安裝的兩個入口）
        if let Some(first) = hits.into_iter().next() {
            out.push(first);
        }
    }
    out
}

/// 把候選路徑依「所在目錄」去重（同目錄視為同一份安裝）。
pub(crate) fn dedupe_by_dir(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen: Vec<PathBuf> = Vec::new();
    let mut out = Vec::new();
    for p in paths {
        let dir = p
            .parent()
            .map(|d| d.canonicalize().unwrap_or_else(|_| d.to_path_buf()))
            .unwrap_or_default();
        if seen.contains(&dir) {
            continue;
        }
        seen.push(dir);
        out.push(p);
    }
    out
}

/// PATH 的所有目錄（保留順序）。
fn path_dirs() -> Vec<PathBuf> {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default()
}

/// 重複安裝診斷（四個工具各一筆）。
///
/// ## `active` 以「目錄」比對，不是以檔名
///
/// [`which_bin`] 在 Windows 上會回**無副檔名**的 shim（`...\npm\claude`），
/// 而 [`find_all_bins_in`] 回的是可執行的那個（`...\npm\claude.cmd`）——
/// 兩者是同一份安裝的不同入口。用完整路徑比對會永遠對不上，
/// 所以只比對所在目錄。
pub fn tool_duplicates() -> Vec<DuplicateReport> {
    let dirs = path_dirs();
    super::APPS
        .iter()
        .map(|(app, display)| {
            let all = dedupe_by_dir(find_all_bins_in(app, &dirs));
            let active_dir = which_bin(&[app]).and_then(|p| {
                p.parent()
                    .map(|d| d.canonicalize().unwrap_or_else(|_| d.to_path_buf()))
            });
            let copies: Vec<ToolCopy> = all
                .iter()
                .map(|p| {
                    let dir = p
                        .parent()
                        .map(|d| d.canonicalize().unwrap_or_else(|_| d.to_path_buf()));
                    ToolCopy {
                        path: p.display().to_string(),
                        active: active_dir.is_some() && dir == active_dir,
                    }
                })
                .collect();
            DuplicateReport {
                app: (*app).to_string(),
                display: (*display).to_string(),
                duplicated: copies.len() > 1,
                copies,
            }
        })
        .collect()
}

/// 一鍵安裝（npm 全域）。只給「尚未安裝」的工具用。
pub fn tool_install_run(app: &str) -> Result<String, String> {
    let pkg = npm_package(app).ok_or_else(|| format!("未知工具：{app}"))?;
    let (installed, _) = probe_tool_version(app);
    if installed {
        return Err(format!("{app} 已經安裝，請用「更新」而不是「安裝」"));
    }
    let argv = vec![
        "cmd".to_string(),
        "/C".to_string(),
        "npm".to_string(),
        "i".to_string(),
        "-g".to_string(),
        pkg.to_string(),
    ];
    let out = run_with_timeout(&argv, 600_000)?;
    let tail: String = out
        .lines()
        .rev()
        .take(20)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n");
    Ok(format!("npm i -g {pkg} 執行完畢：\n{tail}"))
}

/// 全部升級：只處理「已安裝且確實落後」的工具。
///
/// 逐一套用既有的 `tool_update_argv`（與單一更新走同一條路），
/// 每個工具更新後**重新探測版本**，回報「從哪裡到哪裡」。
pub fn tool_update_all() -> Vec<UpdateOutcome> {
    let mut out = Vec::new();
    for (app, display) in super::APPS {
        let (installed, before) = probe_tool_version(app);
        if !installed {
            out.push(UpdateOutcome {
                app: app.to_string(),
                display: display.to_string(),
                ran: false,
                ok: true,
                from: None,
                to: None,
                message: "未安裝，略過".to_string(),
            });
            continue;
        }
        // 先問 registry：問不到就別亂升（可能只是斷網）
        let latest = npm_package(app).and_then(npm_latest);
        let behind = match (&before, &latest) {
            (Some(b), Some(l)) => {
                super::version_cmp::cmp_version(&super::version_cmp::clean_version(b).unwrap_or_else(|| b.clone()), l)
                    == std::cmp::Ordering::Less
            }
            _ => false,
        };
        if !behind {
            let why = match &latest {
                Some(l) => format!("已是最新（{l}），略過"),
                None => "查不到最新版（可能斷網），略過".to_string(),
            };
            out.push(UpdateOutcome {
                app: app.to_string(),
                display: display.to_string(),
                ran: false,
                ok: true,
                from: before,
                to: None,
                message: why,
            });
            continue;
        }
        let (argv, label) = tool_update_argv(app);
        if argv.is_empty() {
            out.push(UpdateOutcome {
                app: app.to_string(),
                display: display.to_string(),
                ran: false,
                ok: false,
                from: before,
                to: None,
                message: "不支援一鍵更新".to_string(),
            });
            continue;
        }
        match run_with_timeout(&argv, 600_000) {
            Ok(_) => {
                let (_, after) = probe_tool_version(app);
                out.push(UpdateOutcome {
                    app: app.to_string(),
                    display: display.to_string(),
                    ran: true,
                    ok: true,
                    from: before,
                    to: after,
                    message: format!("{} 執行完畢", label.unwrap_or_default()),
                });
            }
            Err(e) => out.push(UpdateOutcome {
                app: app.to_string(),
                display: display.to_string(),
                ran: true,
                ok: false,
                from: before,
                to: None,
                message: format!("更新失敗：{e}"),
            }),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn touch(dir: &Path, name: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join(name), b"x").unwrap();
    }

    /// 同一個目錄裡的 cmd 與 ps1 是同一份安裝的兩個入口 —— **不算重複**。
    #[test]
    fn same_dir_shims_are_one_install() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a");
        touch(&a, "claude.cmd");
        #[cfg(windows)]
        touch(&a, "claude.ps1");

        let found = find_all_bins_in("claude", std::slice::from_ref(&a));
        assert_eq!(found.len(), 1, "同目錄只能算一份");
        assert!(found[0].ends_with("claude.cmd") || found[0].ends_with("claude.ps1"));
    }

    /// 兩個不同目錄各有一份 → 重複。
    #[test]
    fn two_dirs_means_duplicated() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a");
        let b = d.path().join("b");
        #[cfg(windows)]
        {
            touch(&a, "codex.exe");
            touch(&b, "codex.exe");
        }
        #[cfg(not(windows))]
        {
            touch(&a, "codex");
            touch(&b, "codex");
        }

        let found = dedupe_by_dir(find_all_bins_in("codex", &[a, b]));
        assert_eq!(found.len(), 2, "不同目錄各一份 = 重複");
    }

    /// 目錄順序＝PATH 優先序，第一個就是實際會被執行的。
    #[test]
    fn order_follows_path_order() {
        let d = tempfile::tempdir().unwrap();
        let first = d.path().join("first");
        let second = d.path().join("second");
        #[cfg(windows)]
        {
            touch(&first, "opencode.cmd");
            touch(&second, "opencode.cmd");
        }
        #[cfg(not(windows))]
        {
            touch(&first, "opencode");
            touch(&second, "opencode");
        }
        let found = find_all_bins_in("opencode", &[first.clone(), second.clone()]);
        assert_eq!(found.len(), 2);
        assert!(found[0].starts_with(&first), "第一個必須是 PATH 上較前面的");
        assert!(found[1].starts_with(&second));
    }

    /// 沒裝就是空的（不要回一個不存在的路徑）。
    #[test]
    fn empty_when_not_installed() {
        let d = tempfile::tempdir().unwrap();
        assert!(find_all_bins_in("nope", &[d.path().to_path_buf()]).is_empty());
        assert!(dedupe_by_dir(vec![]).is_empty());
    }

    /// 已安裝的工具不能再「安裝」。
    #[test]
    fn install_refuses_when_already_installed() {
        // 這台機器上 claude 有沒有裝不確定，所以只驗「未知工具」與「不支援」兩條。
        assert!(tool_install_run("nope").is_err());
        assert_eq!(
            tool_install_run("nope").unwrap_err(),
            "未知工具：nope",
            "未知工具的訊息要講清楚"
        );
    }

    /// 四個工具都要有診斷結果（即使沒裝也要有一筆，前端才能列滿）。
    #[test]
    fn duplicates_report_covers_all_apps() {
        let r = tool_duplicates();
        assert_eq!(r.len(), 4);
        assert!(r.iter().any(|x| x.app == "claude"));
        assert!(r.iter().any(|x| x.app == "codex"));
        // 每一筆的 duplicated 必須與 copies.len() 一致
        for x in &r {
            assert_eq!(x.duplicated, x.copies.len() > 1, "{} 的判定不一致", x.app);
            assert!(x.copies.iter().filter(|c| c.active).count() <= 1, "最多一個 active");
        }
    }
}
