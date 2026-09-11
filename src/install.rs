//! `svnui install-yazi` —— 把插件装进 yazi 配置目录。
//!
//! # 为什么把插件内容嵌进二进制
//!
//! `svnui` 作为 CLI 被装到 `~/.cargo/bin` 后，**不知道源码仓库在哪**。
//! 如果 install 时去相对路径找 `yazi/svnui.yazi/`，那只有在仓库目录里跑才有效。
//!
//! 所以用 `include_str!` 在编译时把插件文件嵌进二进制。代价是二进制大几十 KB，
//! 换来的是"装完就能在任何地方跑 install-yazi"。

use std::path::PathBuf;

use crate::domain::{Error, Result};

/// 插件文件表：`(相对路径, 内容)`。
///
/// ⚠️ 新增插件文件时**必须同步这里**，否则 install 出来的插件会缺文件，
///    而缺 `init.lua` 的表现是"染色不生效但按键有反应"，极难排查。
const FILES: &[(&str, &str)] = &[
    // ⚠️ 只有 main.lua。插件入口文件 **init.lua 已在 yazi #2168 被废弃**，
    //    不要为了"兼容"再加回 init.lua —— 它不会被加载，只会让人以为改它有用。
    //
    // 单文件实现，改完直接 `cargo build`（include_str! 是编译期展开）。
    ("main.lua", include_str!("../yazi/svnui.yazi/main.lua")),
    ("theme.toml", include_str!("../yazi/svnui.yazi/theme.toml")),
    ("keymap.toml", include_str!("../yazi/svnui.yazi/keymap.toml")),
    ("README.md", include_str!("../yazi/svnui.yazi/README.md")),
];

/// 键位片段。不自动写入用户的 keymap.toml —— 见 `print_keymap_hint` 的说明。
const KEYMAP_SNIPPET: &str = include_str!("../yazi/svnui.yazi/keymap.toml");

/// 安装选项。
#[derive(Debug, Clone, Copy, Default)]
pub struct Opts {
    /// 只检查不写盘。
    pub check: bool,
    /// 覆盖已存在的插件文件。
    pub force: bool,
    /// 自动把 `require("svnui"):setup {}` 追加进 init.lua。
    pub patch_init: bool,
}

/// 安装结果。
#[derive(Debug, Default)]
pub struct Report {
    pub plugin_dir: PathBuf,
    pub written: Vec<String>,
    pub skipped: Vec<String>,
    pub init_patched: bool,
    pub init_already: bool,
    pub yazi_config: PathBuf,
}

/// 执行安装。
pub fn install(opts: Opts) -> Result<Report> {
    let config = yazi_config_dir().ok_or_else(|| {
        Error::Parse("找不到 yazi 配置目录。设置 YAZI_CONFIG_HOME 或 HOME。".into())
    })?;

    let plugin_dir = config.join("plugins").join("svnui.yazi");
    let mut rep = Report { plugin_dir: plugin_dir.clone(), yazi_config: config.clone(), ..Default::default() };

    if !opts.check {
        std::fs::create_dir_all(&plugin_dir)?;
    }

    for (rel, body) in FILES {
        let dst = plugin_dir.join(rel);
        if dst.exists() && !opts.force {
            rep.skipped.push(rel.to_string());
            continue;
        }
        if !opts.check {
            std::fs::write(&dst, body)?;
        }
        rep.written.push(rel.to_string());
    }

    // init.lua：检查是否已有 setup 调用
    let init_lua = config.join("init.lua");
    if init_lua.exists() {
        let cur = std::fs::read_to_string(&init_lua).unwrap_or_default();
        if cur.contains("require(\"svnui\")") || cur.contains("require('svnui')") {
            rep.init_already = true;
        } else if opts.patch_init && !opts.check {
            let mut s = cur;
            if !s.ends_with('\n') {
                s.push('\n');
            }
            s.push_str("\n-- svnui: SVN 状态集成（由 svnui install-yazi 自动追加）\n");
            s.push_str("require(\"svnui\"):setup {}\n");
            std::fs::write(&init_lua, s)?;
            rep.init_patched = true;
        }
    }

    Ok(rep)
}

/// yazi 配置目录：`YAZI_CONFIG_HOME` → `~/.config/yazi`（unix）→ `%APPDATA%\yazi`（windows）。
pub fn yazi_config_dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("YAZI_CONFIG_HOME") {
        return Some(PathBuf::from(d));
    }
    if cfg!(windows) {
        let app = std::env::var_os("APPDATA")?;
        return Some(PathBuf::from(app).join("yazi"));
    }
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".config").join("yazi"))
}

/// 键位片段文本。
pub fn keymap_snippet() -> &'static str {
    KEYMAP_SNIPPET
}

/// 渲染人读报告。
pub fn render(r: &Report) -> String {
    let mut out = String::new();
    out.push_str(&format!("yazi 配置  {}\n", r.yazi_config.display()));
    out.push_str(&format!("插件目录  {}\n\n", r.plugin_dir.display()));

    if !r.written.is_empty() {
        out.push_str(&format!("已写入 {} 个文件\n", r.written.len()));
    }
    if !r.skipped.is_empty() {
        out.push_str(&format!("已存在，跳过 {} 个（--force 覆盖）\n", r.skipped.len()));
    }

    out.push('\n');
    if r.init_already {
        out.push_str("init.lua  已含 require(\"svnui\")\n");
    } else if r.init_patched {
        out.push_str("init.lua  已追加 require(\"svnui\"):setup {}\n");
    } else {
        out.push_str("init.lua  ⚠️ 未启用。手动加一行：\n");
        out.push_str("            require(\"svnui\"):setup {}\n");
        out.push_str("          （或重跑 install-yazi --patch-init）\n");
    }

    out.push_str("\n键位见 docs/keymap.md，或跑 `svnui install-yazi --print-keymap`\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_table_is_complete() {
        // 缺 init.lua 的表现是"染色不生效但按键有反应"—— 极难排查，必须有断言守着。
        let names: Vec<&str> = FILES.iter().map(|(n, _)| *n).collect();
        // 每个被 require 的模块都必须在表里 —— 缺一个整个插件就加载不起来
        for must in ["main.lua", "theme.toml", "keymap.toml"] {
            assert!(names.contains(&must), "插件文件表缺 {must}");
        }
    }

    #[test]
    fn embedded_lua_is_not_empty() {
        for (name, body) in FILES {
            assert!(!body.trim().is_empty(), "嵌入的 {name} 是空的 —— include_str! 路径错了？");
        }
    }

    #[test]
    fn keymap_snippet_is_nonempty() {
        assert!(keymap_snippet().contains("plugin svnui"));
    }

    #[test]
    fn check_mode_writes_nothing() {
        let dir = std::env::temp_dir().join(format!("svnui-inst-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::env::set_var("YAZI_CONFIG_HOME", &dir);

        let r = install(Opts { check: true, ..Default::default() }).unwrap();
        // check 模式下 reported 为"将要写入"，但磁盘上不应产生文件
        assert!(!r.plugin_dir.exists(), "--check 不应创建任何文件");

        let _ = std::fs::remove_dir_all(&dir);
        std::env::remove_var("YAZI_CONFIG_HOME");
    }

    #[test]
    fn install_writes_all_files() {
        let dir = std::env::temp_dir().join(format!("svnui-inst2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::env::set_var("YAZI_CONFIG_HOME", &dir);

        let r = install(Opts::default()).unwrap();
        assert_eq!(r.written.len(), FILES.len());
        for (rel, _) in FILES {
            assert!(r.plugin_dir.join(rel).is_file(), "{rel} 未写出");
        }

        // 二次安装应全部跳过（不 force）
        let r2 = install(Opts::default()).unwrap();
        assert_eq!(r2.skipped.len(), FILES.len());
        assert!(r2.written.is_empty());

        // force 后应重新写入
        let r3 = install(Opts { force: true, ..Default::default() }).unwrap();
        assert_eq!(r3.written.len(), FILES.len());

        let _ = std::fs::remove_dir_all(&dir);
        std::env::remove_var("YAZI_CONFIG_HOME");
    }

    #[test]
    fn patch_init_is_idempotent() {
        let dir = std::env::temp_dir().join(format!("svnui-inst3-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("init.lua"), "-- my config\n").unwrap();
        std::env::set_var("YAZI_CONFIG_HOME", &dir);

        let r1 = install(Opts { patch_init: true, ..Default::default() }).unwrap();
        assert!(r1.init_patched);

        // 第二次应识别为"已有"，不重复追加
        let r2 = install(Opts { patch_init: true, ..Default::default() }).unwrap();
        assert!(r2.init_already, "重复 install 不应把 setup 追加两次");

        let s = std::fs::read_to_string(dir.join("init.lua")).unwrap();
        assert_eq!(s.matches("require(\"svnui\")").count(), 1);

        let _ = std::fs::remove_dir_all(&dir);
        std::env::remove_var("YAZI_CONFIG_HOME");
    }

    /// 环境变量在并行测试间会互相干扰，这个测试单独跑（cargo 默认多线程，
    /// 所以用文件名区分目录 + 尽量短的临界区）。真实项目里建议用 `--test-threads=1`
    /// 或把 env 操作收敛到一个测试里。
    #[test]
    fn yazi_config_dir_uses_env() {
        let dir = std::env::temp_dir().join(format!("svnui-cfg-{}", std::process::id()));
        std::env::set_var("YAZI_CONFIG_HOME", &dir);
        assert_eq!(yazi_config_dir(), Some(dir.clone()));
        std::env::remove_var("YAZI_CONFIG_HOME");
    }
}
