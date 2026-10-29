//! 唯一允许 spawn svn 进程的模块。
//!
//! 依赖方向：`client` → `command`，`locator` / `version` 提供环境信息。

pub mod client;
pub mod command;
pub mod locator;
pub mod parser;
pub mod porcelain;
pub mod version;
pub mod workflow;

pub use client::{StatusOpts, Svn};
pub use command::{run, run_with_stdin, RawOutput, RunOpts};
pub use locator::{relative_to, set_svn_exe_from_config, svn_exe, wc_root};
pub use version::SvnVersion;
