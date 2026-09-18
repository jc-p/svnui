//! 统一的等待指示组件。
//!
//! ## 为什么要有这个文件
//!
//! 之前顶栏里是手写的一段拼接：spinner、滑动色块、计时各拼一遍，
//! 别的地方再要"等待中"就得复制一遍。而**复制一遍的结果一定是某一处
//! 忘了同步** —— 有的地方在动、有的地方是静止字符，看着像 bug。
//!
//! 这里把"帧怎么算"和"长什么样"收在一处，调用方只给三样东西：
//! 正在干什么、过了多久、可选的补充信息。
//!
//! ## 帧号为什么不存计数器
//!
//! 帧号只由"过了多少毫秒"推出来，不存计数器。存计数器就得在每次重绘
//! 时 `++`，而重绘时机由事件循环的 poll 决定 —— 一旦某个分支忘了 ++，
//! 动画就卡住不动，表现为一个**静止的色块**（这正是之前 `⟳` 给人的
//! 感觉）。由时间推导则永远在动。
//!
//! ## 两种形态
//!
//! - `spans()`：一行，给顶部状态栏这类"条状"区域。
//! - `render()`：带边框的居中块，给整屏等待用（`Clear` 由调用方负责）。
//!
//! 两种共用同一套帧计算，所以不管出现在哪儿，动起来都是同一个节奏。

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::tui::theme;

/// spinner 的帧序列。
///
/// 盲文点字：字体覆盖好、只占 1 列宽，比 `|/-\` 那套 ASCII 转起来细腻。
const SPINNER_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// spinner 每帧停留多久。
const FRAME_MS: u128 = 100;

/// 滑动色块：轨道总宽 / 滑块宽。
const TRACK_W: usize = 10;
const BLOCK_W: usize = 3;

/// 滑块走一格停留多久。
const SLIDE_MS: u128 = 120;

/// 转动的等待指示。
///
/// 入参是"已经过了多少毫秒"，出参是该时刻该显示的那一帧。
pub fn frame(ms: u128) -> &'static str {
    SPINNER_FRAMES[(ms / FRAME_MS) as usize % SPINNER_FRAMES.len()]
}

/// 往复滑动的色块（不确定进度条）。
///
/// 走到头再折返回来。用 `░` 铺轨道、`▓` 做滑块，**总宽度恒定** ——
/// 顶栏不会因为动画推进而忽长忽短、把右边的字推来推去。
pub fn slide(ms: u128) -> String {
    let span = TRACK_W - BLOCK_W; // 滑块能走到的最远处
    let period = span * 2; // 一去一回
    let t = (ms / SLIDE_MS) as usize % period;
    let start = if t < span { t } else { period - 1 - t };
    format!(
        "{}{}{}",
        "░".repeat(start),
        "▓".repeat(BLOCK_W),
        "░".repeat(TRACK_W - BLOCK_W - start)
    )
}

/// 等待指示。
///
/// `ms` 由调用方算好后传进来（通常是 `start.elapsed().as_millis()`），
/// 组件自己不记时间 —— 记时间就得管生命周期，而 busy 的起止在
/// `App` 里已经统一了，组件只管"给定时刻该长什么样"。
pub struct Loading<'a> {
    label: &'a str,
    ms: u128,
    /// 补充信息，比如检出的"正在落盘第几项"。
    detail: Option<&'a str>,
    /// 是否显示计时。
    with_elapsed: bool,
}

impl<'a> Loading<'a> {
    pub fn new(label: &'a str, ms: u128) -> Self {
        Self {
            label,
            ms,
            detail: None,
            with_elapsed: true,
        }
    }

    /// 补一行细节（如 `1284 项 …A8/xxx.c`）。
    pub fn detail(mut self, d: Option<&'a str>) -> Self {
        self.detail = d;
        self
    }

    /// 关掉计时（空间紧张时用）。
    pub fn without_elapsed(mut self) -> Self {
        self.with_elapsed = false;
        self
    }

    /// 拼成一行，给顶栏这类条状区域。
    ///
    /// 返回 `Vec<Span>` 而不是单个 `Span`：三段各有自己的颜色，
    /// 合成一个 Span 就只能共用一种，计时和细节会跟正文糊在一起。
    pub fn spans(&self) -> Vec<Span<'static>> {
        let mut v = vec![
            Span::styled(format!(" {} ", frame(self.ms)), theme::Theme::props()),
            Span::styled(format!("{} ", slide(self.ms)), theme::Theme::props()),
            Span::styled(format!("{} …", self.label), theme::Theme::title()),
        ];

        if let Some(d) = self.detail {
            if !d.is_empty() {
                v.push(Span::styled(format!(" {}", d), theme::Theme::dim()));
            }
        }

        if self.with_elapsed {
            v.push(Span::styled(
                format!(" {:.1}s ", self.ms as f64 / 1000.0),
                theme::Theme::props(),
            ));
        }

        v
    }

    /// 一行版（不需要拆 Span 时用）。
    pub fn line(&self) -> Line<'static> {
        Line::from(self.spans())
    }

    /// 带边框的居中块。
    ///
    /// 给"整屏只剩等待"的场景（比如还没有工作副本、正在拉大仓库）。
    /// `Clear` 不在这里画 —— 调用方才知道底下盖着什么。
    pub fn render(&self, f: &mut Frame, area: Rect) {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(Line::from(Span::styled(
                format!(" {} ", self.label),
                theme::Theme::title(),
            )))
            .border_style(theme::Theme::border_active());

        let inner = block.inner(area);
        f.render_widget(block, area);

        // 三行：动画 / 细节 / 提示。细节可能没有，就留空。
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Min(0),
            ])
            .split(inner);

        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(format!(" {} ", frame(self.ms)), theme::Theme::props()),
                Span::styled(slide(self.ms), theme::Theme::props()),
                Span::styled(
                    if self.with_elapsed {
                        format!("  {:.1}s", self.ms as f64 / 1000.0)
                    } else {
                        String::new()
                    },
                    theme::Theme::dim(),
                ),
            ]))
            .alignment(Alignment::Center),
            rows[0],
        );

        let detail = self.detail.unwrap_or("");
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(detail, theme::Theme::text())))
                .alignment(Alignment::Center),
            rows[1],
        );

        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                " 进行中，请稍候（Esc 不会中断已启动的操作） ",
                theme::Theme::dim(),
            )))
            .alignment(Alignment::Center),
            rows[2],
        );
    }
}
