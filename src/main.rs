//! `svnui` 二进制入口。**只做三件事**：解析参数、调用分发、把结果翻译成退出码。
//!
//! 所有业务逻辑在 `svnui::cli`，领域模型在 `svnui::domain`。

use clap::Parser;

fn main() {
    let cli = svnui::cli::Cli::parse();
    std::process::exit(svnui::cli::run(cli));
}
