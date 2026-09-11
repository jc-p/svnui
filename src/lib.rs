//! `svnui` —— Subversion 命令的 Rust 封装层。
//!
//! 分层与依赖方向（单向，越往下越底层）：
//!
//! ```text
//! main → cli → svn::client → svn::command
//!                  ↓
//!               domain   （纯模型，零 IO，谁都能依赖）
//! ```
//!
//! 三条硬约束：
//! 1. `domain` 零 IO —— 只放数据结构与纯函数。
//! 2. `svn` 是唯一允许 spawn 进程的地方。
//! 3. `output` 负责所有格式化，其他模块不关心终端。

pub mod cache;
pub mod cli;
pub mod daemon;
pub mod domain;
pub mod install;
pub mod ipc;
pub mod output;
pub mod policy;
pub mod svn;

/// TUI 层。feature = "tui" 时才有。
///
/// 依赖方向：tui → {domain, svn}，和 cli 平级，互不依赖。
#[cfg(feature = "tui")]
pub mod tui;

pub use domain::{Error, Result};

/// 重导出 camino。`StatusEntry.path` 等 pub 字段是 camino 类型，
/// 调用方要构造它们时必须能拿到**同一版本**的类型 —— 不重导出的话，
/// 下游只能自己加 camino 依赖，版本不一致时会出现难以理解的类型不匹配。
pub use camino;
