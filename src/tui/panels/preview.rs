//! 右侧预览面板。
//!
//! **按选中项的状态决定显示什么** —— 这是它和单纯 diff 面板的区别：
//!
//! | 选中 | 显示 |
//! |---|---|
//! | 目录 | 子项统计 + 状态汇总 |
//! | 有改动（M/A/D/R/C/T/!） | 彩色 diff |
//! | 未版本化（?） | 文件内容 + 顶部提示 |
//! | 干净文件 | 文件内容 |
//!
//! 干净文件也能预览很重要：浏览代码时不用为了看一眼内容就退出。
//!
//! 同一类型既用于右栏内嵌（`render_pane`），也用于全屏弹框（`render`）。

use std::path::Path;

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState},
    Frame,
};

use crate::tui::theme;

/// 预览内容从哪来。影响标题措辞和是否需要"先 add"这类提示。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    Diff,
    File,
    Notice,
}

/// 预览面板。
pub struct PreviewPanel {
    kind: Kind,
    title: String,
    lines: Vec<Line<'static>>,
    /// 顶部横幅（提示语）。占内容区第一行，**不随滚动移动**。
    banner: Option<String>,
    scroll: u16,
    /// 横向滚动偏移（列）。长行超出右边界时用。
    h_scroll: u16,
}

/// 单个预览最多显示多少行。
///
/// 几万行的文件全解析成 Line 会让 hover 明显卡住。
const MAX_LINES: usize = 5000;

impl PreviewPanel {
    // ── 构造 ──────────────────────────────────────────────

    /// 彩色 diff。按行首字符着色。
    pub fn diff(title: impl Into<String>, raw: &str) -> Self {
        let lines: Vec<Line<'static>> = raw.lines().take(MAX_LINES).map(colorize_diff).collect();
        Self::with_lines(Kind::Diff, title, lines, None)
    }

    /// 文件内容。行号暗色显示，正文不套 diff 着色规则。
    pub fn file(title: impl Into<String>, content: &str) -> Self {
        let total = content.lines().count();
        let lines: Vec<Line<'static>> = content
            .lines()
            .take(MAX_LINES)
            .enumerate()
            .map(|(i, l)| {
                Line::from(vec![
                    Span::styled(format!("{:>5} ", i + 1), theme::Theme::dim()),
                    Span::raw(l.replace('\t', "    ")),
                ])
            })
            .collect();

        let banner = if total > MAX_LINES {
            Some(format!(" 只显示前 {} 行（共 {} 行）", MAX_LINES, total))
        } else {
            None
        };
        Self::with_lines(Kind::File, title, lines, banner)
    }

    /// 提示（不是 diff 也不是文件内容）。
    pub fn notice(title: impl Into<String>, msg: impl Into<String>) -> Self {
        Self::with_lines(
            Kind::Notice,
            title,
            vec![
                Line::from(""),
                Line::from(Span::styled(format!("  {}", msg.into()), theme::Theme::dim())),
            ],
            None,
        )
    }

    /// 同 `notice`，但额外带一条横幅（用于"未版本化，按 A add"这类）。
    pub fn file_with_banner(
        title: impl Into<String>,
        content: &str,
        banner: impl Into<String>,
    ) -> Self {
        let mut p = Self::file(title, content);
        p.banner = Some(banner.into());
        p
    }

    /// 直接用现成的着色行构造（历史版本 diff 用）。
    pub fn with_lines(
        kind: Kind,
        title: impl Into<String>,
        lines: Vec<Line<'static>>,
        banner: Option<String>,
    ) -> Self {
        Self {
            kind,
            title: title.into(),
            lines,
            banner,
            scroll: 0,
            h_scroll: 0,
        }
    }

    pub fn kind(&self) -> Kind {
        self.kind
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    // ── 滚动 ──────────────────────────────────────────────

    pub fn scroll_down(&mut self, n: u16) {
        self.scroll = self.scroll.saturating_add(n);
    }

    pub fn scroll_up(&mut self, n: u16) {
        self.scroll = self.scroll.saturating_sub(n);
    }

    pub fn scroll_top(&mut self) {
        self.scroll = 0;
    }

    /// 横向滚动。正数向右，负数向左。
    pub fn scroll_right(&mut self, n: u16) {
        self.h_scroll = self.h_scroll.saturating_add(n);
    }

    pub fn scroll_left(&mut self, n: u16) {
        self.h_scroll = self.h_scroll.saturating_sub(n);
    }

    /// 回到最左（列 0）。
    pub fn scroll_home(&mut self) {
        self.h_scroll = 0;
    }

    /// 最长行的显示宽度。横向滚动条和 clamp 都要用。
    ///
    /// 按字符数算而不是字节数 —— 中文一个字占 2 列但 3 字节。
    /// 这里用 chars().count() 是近似值（中文实际上更宽），
    /// 但对"能不能往右滚"这个判断足够，且不用引 unicode-width。
    fn max_line_width(&self) -> usize {
        self.lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|sp| sp.content.chars().count())
                    .sum::<usize>()
            })
            .max()
            .unwrap_or(0)
    }

    fn clamp_scroll(&mut self, visible_h: u16) {
        let max = self.lines.len().saturating_sub(visible_h as usize) as u16;
        if self.scroll > max {
            self.scroll = max;
        }
    }

    // ── 渲染 ──────────────────────────────────────────────

    /// 内嵌在右栏。
    pub fn render_pane(&mut self, f: &mut Frame, area: Rect) {
        let label = match self.kind {
            Kind::Diff => "改动",
            Kind::File => "预览",
            Kind::Notice => "信息",
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .title(format!(" {} · {} ", label, self.title))
            .border_style(theme::Theme::border());
        self.paint(f, area, block);
    }

    /// 全屏弹框。
    pub fn render(&mut self, f: &mut Frame, area: Rect) {
        f.render_widget(Clear, area);

        let block = Block::default()
            .borders(Borders::ALL)
            .title(format!(" {} （Esc 返回）", self.title))
            .title_bottom(if self.lines.is_empty() {
                Line::from(" （无内容）").alignment(Alignment::Center)
            } else {
                Line::from(format!(
                    " {} 行 | j/k 滚动 | d/u 翻页 | g 回顶部 ",
                    self.lines.len()
                ))
                .alignment(Alignment::Right)
            })
            .border_style(theme::Theme::border_active());

        self.paint(f, area, block);
    }

    /// 画框 → 横幅（占内容第一行，不滚动）→ 正文（可滚动）。
    fn paint(&mut self, f: &mut Frame, area: Rect, block: Block<'_>) {
        let inner = block.inner(area);
        f.render_widget(block, area);

        // 有横幅就把内容区切出一行的位置给它
        let (body_area, visible_h) = match &self.banner {
            Some(b) if inner.height >= 2 => {
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Length(1), Constraint::Min(1)])
                    .split(inner);
                f.render_widget(
                    Paragraph::new(Line::from(Span::styled(b.as_str(), theme::Theme::props()))),
                    chunks[0],
                );
                (chunks[1], chunks[1].height)
            }
            _ => (inner, inner.height),
        };

        self.clamp_scroll(visible_h);

        // 横向也要 clamp：内容变短（切到别的文件）后，
        // 旧的 h_scroll 会让画面停在一片空白上。
        let max_w = self.max_line_width() as u16;
        let visible_w = body_area.width.saturating_sub(1); // 留出竖向滚动条的列
        if self.h_scroll > max_w.saturating_sub(visible_w) {
            self.h_scroll = max_w.saturating_sub(visible_w);
        }

        f.render_widget(
            Paragraph::new(Text::from(self.lines.clone())).scroll((self.scroll, self.h_scroll)),
            body_area,
        );

        // 竖向滚动条
        if self.lines.len() as u16 > visible_h {
            let mut sb = ScrollbarState::new(self.lines.len()).position(self.scroll as usize);
            f.render_stateful_widget(
                Scrollbar::new(ScrollbarOrientation::VerticalRight),
                body_area,
                &mut sb,
            );
        }

        // 横向滚动条：只在真的有内容超宽时出现。
        //
        // 用细横线（━）而不是默认实心块（█）：
        // 默认样式是一条粗色带，压在内容底部很突兀，
        // 而且会让人误以为那是分隔线或高亮行。
        // begin/end symbol 设成 None 去掉首尾箭头，进一步收窄。
        if max_w > visible_w {
            let mut sb = ScrollbarState::new(max_w as usize).position(self.h_scroll as usize);
            f.render_stateful_widget(
                Scrollbar::new(ScrollbarOrientation::HorizontalBottom)
                    .thumb_symbol("━")
                    .track_symbol(None)
                    .begin_symbol(None)
                    .end_symbol(None),
                body_area,
                &mut sb,
            );
        }
    }
}

/// diff 着色。`+` 绿、`-` 红、`@` 青、其余压暗。
fn colorize_diff(l: &str) -> Line<'static> {
    let style = if l.starts_with("+++") || l.starts_with("---") {
        theme::Theme::dim()
    } else if l.starts_with('+') {
        theme::Theme::added()
    } else if l.starts_with('-') {
        theme::Theme::deleted()
    } else if l.starts_with("@@") {
        theme::Theme::props()
    } else if l.starts_with("===") || l.starts_with("Index:") {
        theme::Theme::props()
    } else {
        theme::Theme::dim()
    };
    Line::from(Span::styled(l.to_string(), style))
}

/// 单个文件最多读多少字节。超了截断并提示。
const MAX_BYTES: u64 = 2 * 1024 * 1024;

/// 读文件用于预览。
///
/// 三件事必须处理，否则终端会被搞乱：
/// 1. **二进制** —— 把图片/压缩包打进终端会输出一堆控制字符，
///    甚至改变终端状态。靠前 8KB 里有没有 NUL 判定。
/// 2. **超大文件** —— 2MB 以上截断。
/// 3. **非 UTF-8** —— lossy 转换而不是报错。
pub fn read_file(path: &Path) -> Result<String, std::io::Error> {
    use std::io::Read;

    // ── 办公文档：必须在下面的二进制检测之前拦截 ──
    //
    // .xlsx/.docx 是 zip 包，前 8KB 里一定有 NUL。如果不先分派给
    // document::extract，它们会被二进制检测当成"图片/压缩包"直接挡掉。
    #[cfg(feature = "docs")]
    {
        if crate::tui::panels::document::is_document(path) {
            return match crate::tui::panels::document::extract(path) {
                Some(text) => Ok(text),
                // 走到这里说明扩展名对但内容解析不了：文件损坏、
                // 被 Excel/Word 锁着、或者格式比库支持的更新。
                None => Ok("（无法解析该文档：文件可能已损坏、正被占用，或格式不受支持）\n\n按 E 用外部程序打开".to_string()),
            };
        }
    }

    let meta = std::fs::metadata(path)?;
    if !meta.is_file() {
        return Ok(String::new());
    }

    if meta.len() > MAX_BYTES {
        let mut f = std::fs::File::open(path)?;
        let mut buf = vec![0u8; MAX_BYTES as usize];
        let n = f.read(&mut buf)?;
        buf.truncate(n);
        let text = String::from_utf8_lossy(&buf).to_string();
        return Ok(format!(
            "{}\n\n… 文件过大（{} 字节），只显示前 {} 字节",
            text, meta.len(), MAX_BYTES
        ));
    }

    let mut buf = Vec::with_capacity(meta.len() as usize);
    std::fs::File::open(path)?.read_to_end(&mut buf)?;

    // 二进制检测：前 8KB 里有 NUL 就当二进制
    let probe_end = buf.len().min(8192);
    if buf[..probe_end].contains(&0) {
        return Ok("（二进制文件，不显示内容）".to_string());
    }

    Ok(String::from_utf8_lossy(&buf).to_string())
}
