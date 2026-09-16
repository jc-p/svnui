//! 输出层：把领域数据渲染成 human / json / porcelain。
//!
//! 全局契约（调用方只认这个）：
//! - JSON 一律信封 `{"ok":true,"data":...}` / `{"ok":false,"error":{"kind":..,"message":..}}`
//! - 非 TTY 时 human 输出**不带** ANSI 颜色，避免污染管道。

pub mod json;
pub mod pager;

pub use json::{err, kind_of, ok};
pub use pager::{page, page_if_long, PagerKind};

/// 是否为交互式终端。管道/重定向时返回 false。
pub fn is_tty() -> bool {
    use std::io::IsTerminal;
    std::io::stdout().is_terminal()
}

/// 极简 ANSI 封装。刻意不引 colored —— 少一个依赖，且我们只需要几个颜色。
pub mod style {
    pub fn paint(code: &str, s: &str, tty: bool) -> String {
        if !tty {
            return s.to_string();
        }
        format!("\x1b[{code}m{s}\x1b[0m")
    }

    pub fn green(s: &str, tty: bool) -> String {
        paint("32", s, tty)
    }
    pub fn yellow(s: &str, tty: bool) -> String {
        paint("33", s, tty)
    }
    pub fn red(s: &str, tty: bool) -> String {
        paint("31", s, tty)
    }
    #[allow(dead_code)]
    pub fn cyan(s: &str, tty: bool) -> String {
        paint("36", s, tty)
    }
    pub fn magenta(s: &str, tty: bool) -> String {
        paint("35", s, tty)
    }
    #[allow(dead_code)]
    pub fn gray(s: &str, tty: bool) -> String {
        paint("90", s, tty)
    }
    pub fn bold(s: &str, tty: bool) -> String {
        paint("1", s, tty)
    }

    /// 按状态符号选颜色 —— 与 调用方的 theme.toml 保持一致的语义。
    pub fn by_sign(sign: char, s: &str, tty: bool) -> String {
        match sign {
            'A' | 'G' => green(s, tty),
            'M' => yellow(s, tty),
            'D' | '!' => red(s, tty),
            'C' | 'T' => paint("1;31", s, tty),
            'R' | 'S' => magenta(s, tty),
            'X' | 'K' => paint("34", s, tty),
            'I' | '?' => paint("90", s, tty),
            '~' => paint("91", s, tty),
            _ => s.to_string(),
        }
    }
}
