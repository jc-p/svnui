//! `svnui` 二进制入口。**只做三件事**：解析参数、调用分发、把结果翻译成退出码。
//!
//! 所有业务逻辑在 `svnui::cli`，领域模型在 `svnui::domain`。

use clap::Parser;

fn main() {
    let cli = svnui::cli::Cli::parse();

    // 首次运行生成一份带注释的配置，省得翻文档找字段。
    // 只在文件不存在时写，已存在一律不动；写不了就当没这回事。
    // 提示走 stderr —— stdout 要留给脚本解析。
    let llm_command = matches!(cli.cmd, Some(svnui::cli::Cmd::Llm { .. }));
    if !llm_command && svnui::config::Config::ensure_file() {
        if let Some(p) = svnui::config::Config::path() {
            eprintln!(
                "已生成配置文件 {}（每个字段都有注释，可直接改）",
                p.display()
            );
        }
    }

    std::process::exit(svnui::cli::run(cli));
}
