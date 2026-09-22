//! 检出新工作副本。
//!
//! 在没有工作副本的目录里启动 `svnui tui` 时会进到这里 ——
//! 否则用户得先退出、在命令行敲 checkout、再进来。

use std::path::PathBuf;

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

use crate::tui::theme;

/// 字段顺序。
const FIELDS: &[(&str, bool)] = &[
    ("仓库 URL", false),
    ("本地路径", false),
    ("用户名", false),
    ("密码", true), // 掩码显示
];

/// svn 已缓存的凭证条数（`~/.subversion/auth/svn.simple/`）。
///
/// svn 的凭据缓存在**第一次需要认证的操作**里顺带写入，之后同类操作
/// 不用再输。所以"本地已经有凭证"是完全可能的状态 —— 面板得让用户
/// 知道这一点，否则每次都以为必须手填。
///
/// 读不到就当 0（目录不存在 / 权限问题 / 用了非默认 config dir）。
/// 这里只做**计数**，不预填用户名：多个仓库可能用不同账号，
/// 填错比不填更糟 —— 不填时 svn 会自己按 realm 挑一条。
pub fn cached_credential_count() -> usize {
    // svn 自己的规则：有 SVN_CONFIG_DIR 就用它，否则 ~/.subversion。
    let base: Option<PathBuf> = match std::env::var_os("SVN_CONFIG_DIR") {
        Some(d) => Some(PathBuf::from(d)),
        None => std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".subversion")),
    };
    let Some(base) = base else { return 0 };

    let dir = base.join("auth").join("svn.simple");
    std::fs::read_dir(&dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter(|e| e.path().is_file())
                .count()
        })
        .unwrap_or(0)
}

pub struct CheckoutPanel {
    values: [String; 4],
    focus: usize,
    hint: Option<String>,
    /// 进入面板时探测到的 svn 凭证缓存条数。
    cached: usize,
}

impl CheckoutPanel {
    pub fn new(default_path: String) -> Self {
        Self {
            values: [String::new(), default_path, String::new(), String::new()],
            focus: 0,
            hint: None,
            cached: cached_credential_count(),
        }
    }

    pub fn set_hint(&mut self, h: impl Into<String>) {
        self.hint = Some(h.into());
    }

    /// 直接覆盖第 `i` 个字段（0=URL / 1=本地路径 / 2=用户名 / 3=密码）。
    ///
    /// 给 Repo 面板"检出这个目录"用 —— URL 已知，没必要再拼一次。
    /// 只接受下标 0..3，越界直接忽略：调用方传错不该 panic。
    pub fn set_field(&mut self, i: usize, s: &str) {
        if let Some(v) = self.values.get_mut(i) {
            *v = s.to_string();
            self.focus = i;
        }
    }

    pub fn url(&self) -> &str {
        &self.values[0]
    }

    pub fn path(&self) -> &str {
        &self.values[1]
    }

    pub fn username(&self) -> Option<&str> {
        let s = self.values[2].trim();
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    }

    pub fn password(&self) -> Option<&str> {
        let s = &self.values[3];
        if s.is_empty() {
            None
        } else {
            Some(s.as_str())
        }
    }

    /// 切到下一个字段（密码后可回车）。
    pub fn next_field(&mut self) {
        self.focus = (self.focus + 1) % FIELDS.len();
    }

    pub fn prev_field(&mut self) {
        self.focus = (self.focus + FIELDS.len() - 1) % FIELDS.len();
    }

    pub fn focus(&self) -> usize {
        self.focus
    }

    pub fn push_char(&mut self, c: char) {
        self.values[self.focus].push(c);
    }

    /// 整段插入（粘贴）。
    ///
    /// 单行输入框吃不下换行：URL 里带 `\n` 会把框撑成两行、边框错位，
    /// 所以统一压成空格。制表符同理（Tab 是切字段的键，不能出现在内容里）。
    pub fn push_str(&mut self, s: &str) {
        let flat: String = s
            .chars()
            .map(|c| if c == '\n' || c == '\r' || c == '\t' { ' ' } else { c })
            .collect();
        self.values[self.focus].push_str(&flat);
    }

    pub fn pop_char(&mut self) {
        self.values[self.focus].pop();
    }

    /// 清空当前字段（Ctrl+U）。粘错一长串 URL 时逐字符退格太慢。
    pub fn clear_field(&mut self) {
        self.values[self.focus].clear();
    }

    pub fn render(&self, f: &mut Frame, area: Rect) {
        f.render_widget(Clear, area);

        // 外层再包一个边框：字段框自己有边框，外面这一层把它们收成一张
        // "表单卡片"，跟底下的主界面分开。标题放进 Block 的 title，
        // 比单独占一行省地方，也不会跟字段框的标题混淆层级。
        let outer = Block::default()
            .borders(Borders::ALL)
            .title(Line::from(Span::styled(
                " 检出一份工作副本 ",
                theme::Theme::title(),
            )))
            .border_style(theme::Theme::border_active());
        let inner = outer.inner(area);
        f.render_widget(outer, area);

        // 每个字段必须是 3 行：Block 的上下边框各占 1 行，
        // 留给文本的内容区只有中间那 1 行。
        //
        // ⚠️ 这里原来是 Length(2) —— 边框吃光后内容区高度是 **0**，
        //    于是输入的值、placeholder、连焦点光标 ▌ 全都被裁掉。
        //    值其实存进去了（提交时能看到），只是画不出来，
        //    表现为"框是空的 / 打不出字"。
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // URL
                Constraint::Length(3), // path
                Constraint::Length(3), // user
                Constraint::Length(3), // pass
                Constraint::Length(1), // 凭证状态
                Constraint::Min(0),    // 提示（吸收剩余）
            ])
            .split(inner);

        for (i, (label, masked)) in FIELDS.iter().enumerate() {
            let active = i == self.focus;
            let shown = if *masked {
                "*".repeat(self.values[i].chars().count())
            } else {
                self.values[i].clone()
            };

            let block = Block::default()
                .borders(Borders::ALL)
                .title(format!(" {} ", label))
                .border_style(if active {
                    theme::Theme::border_active()
                } else {
                    theme::Theme::border()
                });

            // ⚠️ 先算好再 format!：把 `shown` 直接塞进三元表达式会被 move，
            //    下面还要用它判断颜色，就借不到了。
            let placeholder = shown.is_empty() && !active;
            let content = if placeholder {
                // 用户名/密码：明确写出"留空就用缓存"，比"可留空"清楚。
                let tail = match (i, *masked) {
                    (2, _) => "，留空用已缓存凭证",
                    (3, true) => "，留空用已缓存凭证",
                    _ => "",
                };
                format!("（{}{}）", label, tail)
            } else {
                shown
            };
            // 焦点框里补一个可见光标。
            let content = if active && !placeholder {
                format!("{}▌", content)
            } else {
                content
            };

            // 输入的内容用白字 —— 之前用的是 status_bar()（黑字青底），
            // 那片青底会把字压住；placeholder 保持暗色，跟真值区分开。
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    format!(" {}", content),
                    if placeholder {
                        theme::Theme::dim()
                    } else if active {
                        theme::Theme::input_active()
                    } else {
                        theme::Theme::input()
                    },
                )))
                .block(block),
                rows[i],
            );
        }

        // 凭证状态单独一行 —— 这是"到底要不要手填"的依据。
        let cred = if self.cached > 0 {
            Line::from(Span::styled(
                format!(
                    " 已检测到 {} 条 svn 凭证缓存 —— 用户名/密码直接留空即可，svn 会自动匹配",
                    self.cached
                ),
                theme::Theme::dim(),
            ))
        } else {
            Line::from(Span::styled(
                " 未检测到 svn 凭证缓存 —— 需填写用户名/密码（或先跑 svnui login 缓存一次）",
                theme::Theme::conflicted(),
            ))
        };
        f.render_widget(Paragraph::new(cred).alignment(Alignment::Left), rows[4]);

        let hint = match &self.hint {
            Some(h) => Line::from(Span::styled(h.as_str(), theme::Theme::conflicted())),
            None => Line::from(Span::styled(
                " Tab 切换   Enter 开始检出（在最后一项按 Enter 直接开始）   Ctrl+U 清空   Esc 退出   可直接粘贴",
                theme::Theme::dim(),
            )),
        };
        f.render_widget(Paragraph::new(hint).alignment(Alignment::Left), rows[5]);
    }
}
