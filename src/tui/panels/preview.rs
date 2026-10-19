//! 右侧预览面板。
//!
//! 先按类型及大小分流：图片独立绘制，压缩包/大文件显示提示；普通文本按状态预览：
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

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState},
    Frame,
};
use ratatui_image::{protocol::StatefulProtocol, StatefulImage};
use crate::tui::image_support::ImageSupport;

use crate::tui::theme;

/// 预览内容从哪来。影响标题措辞和是否需要"先 add"这类提示。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    Diff,
    File,
    Notice,
    Image,
}

/// 滚动条命中结果。拖拽滚动条时用。
///
/// 竖向条（最右一列）和横向条（最底一行）分开：
/// 两者映射的目标不同（行 vs 列），混在一起没法算。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BarHit {
    Vertical,
    Horizontal,
}

/// 预览面板。
pub struct PreviewPanel {
    kind: Kind,
    title: String,
    lines: Vec<Line<'static>>,
    /// 内容构造时计算，避免每帧遍历全部文本。
    max_width: u16,
    /// 顶部横幅（提示语）。占内容区第一行，**不随滚动移动**。
    banner: Option<String>,
    image: Option<StatefulProtocol>,
    scroll: u16,
    /// 横向滚动偏移（列）。长行超出右边界时用。
    h_scroll: u16,
    /// 上一次渲染时的可视宽度（列）。
    ///
    /// 「横向翻页」要按屏宽算步长 —— 写死 8 列的话，
    /// 宽屏上要按十几次才能翻完一屏，跟"翻页"这个名字不符。
    /// 面板自己最清楚画出来有多宽，所以在 paint 里记一笔，
    /// 外部（键盘/鼠标）翻页时直接问它要步长。
    vis_w: u16,
    /// 上一次渲染时的可视高度（行）。竖向滚动位置的最大值要用。
    vis_h: u16,
    /// 滚动条独占的区域；正文不绘制到这些格子里，拖拽也使用同一范围。
    v_bar_area: Rect,
    h_bar_area: Rect,
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

    /// 只读预览的截断提示独立显示，不混入 diff 正文。
    pub(crate) fn diff_with_limits(
        title: impl Into<String>,
        raw: &str,
        stdout_truncated: bool,
        stderr_truncated: bool,
    ) -> Self {
        let mut panel = Self::diff(title, raw);
        let lines_truncated = raw.lines().nth(MAX_LINES).is_some();
        if stdout_truncated || lines_truncated {
            panel.banner = Some(format!(
                "预览已截断 · 最多显示 {MAX_LINES} 行 · 返回文件树后按 E 编辑或 o 系统打开查看完整文件"
            ));
        } else if stderr_truncated {
            panel.banner = Some("SVN 诊断输出已截断 · diff 正文完整".into());
        }
        panel
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

    /// 解码在后台完成；缩略图限制后续按窗口尺寸编码时的开销。
    pub(crate) fn image(title: impl Into<String>, pixels: image::DynamicImage, bytes: u64, picker: &ImageSupport) -> Self {
        let (width, height) = (pixels.width(), pixels.height());
        let mut panel = Self::with_lines(Kind::Image, title, Vec::new(), Some(format!(
            "{width} × {height} · {bytes} 字节 · 自动适应窗口"
        )));
        let pixels = if width > 1600 || height > 1600 {
            pixels.thumbnail(1600, 1600)
        } else {
            pixels
        };
        panel.image = Some(picker.new_resize_protocol(pixels));
        panel
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
        let max_width = lines.iter().map(Line::width).max().unwrap_or(0)
            .min(u16::MAX as usize) as u16;
        Self {
            max_width,
            kind,
            title: title.into(),
            lines,
            banner,
            image: None,
            scroll: 0,
            h_scroll: 0,
            vis_w: 0,
            vis_h: 0,
            v_bar_area: Rect::default(),
            h_bar_area: Rect::default(),
        }
    }

    pub fn kind(&self) -> Kind {
        self.kind
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty() && self.image.is_none()
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

    /// 横向翻页步长：约 3/4 屏宽，至少 8 列。
    ///
    /// 不到整屏是为了留一点重叠 —— 翻过去还能看到上一页末尾两三个词，
    /// 不然接缝处上下文直接断掉，读 diff 时很别扭。
    pub fn page_step(&self) -> u16 {
        let w = self.vis_w.saturating_sub(2);
        std::cmp::max(8, w * 3 / 4)
    }

    /// 纵向翻页保留一行重叠，方便衔接上下文。
    pub fn vertical_page_step(&self) -> u16 {
        self.vis_h.saturating_sub(1).max(1)
    }

    /// 按终端显示列计算宽度，中文等宽字符同样计入横向滚动范围。
    fn max_line_width(&self) -> u16 {
        self.max_width
    }

    fn clamp_scroll(&mut self, visible_h: u16) {
        let max = self.lines.len().saturating_sub(visible_h as usize) as u16;
        if self.scroll > max {
            self.scroll = max;
        }
    }

    // ── 滚动条拖拽 ────────────────────────────────────────

    /// 竖向最大滚动位置（行）。
    pub fn max_scroll_v(&self) -> u16 {
        self.lines.len().saturating_sub(self.vis_h as usize) as u16
    }

    /// 横向最大滚动位置（列）。
    fn max_scroll_h(&self) -> u16 {
        self.max_line_width().saturating_sub(self.vis_w)
    }

    /// 只命中独立轨道，右下角空白和正文不参与拖拽。
    pub fn hit_bar(&self, x: u16, y: u16) -> Option<BarHit> {
        let v = self.v_bar_area;
        if v.width > 0 && x == v.x && y >= v.y && y < v.bottom() {
            return Some(BarHit::Vertical);
        }
        let h = self.h_bar_area;
        if h.height > 0 && y == h.y && x >= h.x && x < h.right() {
            return Some(BarHit::Horizontal);
        }
        None
    }

    /// 把竖向条上的 y 坐标映射成滚动位置。
    ///
    /// 按比例映射而不是"移动了多少就滚多少"：
    /// 后者在长文件里拖到底都滚不完，滑块和鼠标会越离越远。
    pub fn drag_v(&mut self, y: u16) {
        let a = self.v_bar_area;
        let max = self.max_scroll_v();
        if a.height == 0 || max == 0 {
            return;
        }
        let span = (a.height - 1).max(1) as u64;
        let rel = y.saturating_sub(a.y) as u64;
        self.scroll = ((rel * max as u64) / span).min(max as u64) as u16;
    }

    /// 把横向条上的 x 坐标映射成滚动位置。
    pub fn drag_h(&mut self, x: u16) {
        let a = self.h_bar_area;
        let max = self.max_scroll_h();
        if a.width == 0 || max == 0 {
            return;
        }
        let span = (a.width - 1).max(1) as u64;
        let rel = x.saturating_sub(a.x) as u64;
        self.h_scroll = ((rel * max as u64) / span).min(max as u64) as u16;
    }

    // ── 渲染 ──────────────────────────────────────────────

    /// 内嵌在右栏。
    pub fn render_pane(&mut self, f: &mut Frame, area: Rect, focused: bool) {
        let label = match self.kind {
            Kind::Diff => "改动",
            Kind::File => "预览",
            Kind::Notice => "信息",
            Kind::Image => "图片",
        };
        let block = theme::block()
            .borders(Borders::ALL)
            .title(format!(" {} · {} ", label, self.title))
            .title_bottom(if focused { " 预览操作 · Tab/Esc 返回文件树 " } else { " Tab 激活预览 " })
            .border_style(if focused { theme::Theme::border_active() } else { theme::Theme::border() });
        self.paint(f, area, block);
    }

    /// 全屏弹框。
    pub fn render(&mut self, f: &mut Frame, area: Rect) {
        f.render_widget(Clear, area);

        let block = theme::block()
            .borders(Borders::ALL)
            .title(format!(" {} （Esc 返回）", self.title))
            .title_bottom(if self.image.is_some() {
                Line::from(" 图片 · 自动适应窗口 · Esc 返回 ")
            } else if self.lines.is_empty() {
                Line::from(" （无内容）").alignment(Alignment::Center)
            } else {
                Line::from(format!(
                    " {} 行 | j/k 滚动 | d/u 翻页 | ← → 慢移 | < > 翻页 | g 回顶部 | 拖滚动条 ",
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
        let body_area = match &self.banner {
            Some(b) if inner.height >= 2 => {
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Length(1), Constraint::Min(1)])
                    .split(inner);
                f.render_widget(
                    Paragraph::new(Line::from(Span::styled(b.as_str(), theme::Theme::props()))),
                    chunks[0],
                );
                chunks[1]
            }
            _ => inner,
        };

        if let Some(image) = self.image.as_mut() {
            self.v_bar_area = Rect::default();
            self.h_bar_area = Rect::default();
            self.vis_w = body_area.width;
            self.vis_h = body_area.height;
            self.scroll = 0;
            self.h_scroll = 0;
            f.render_stateful_widget(StatefulImage::default(), body_area, image);
            if let Some(Err(error)) = image.last_encoding_result() {
                self.image = None;
                self.kind = Kind::Notice;
                self.banner = None;
                self.lines = vec![Line::from(format!("无法绘制图片：{error}"))];
                self.max_width = self.lines[0].width().min(u16::MAX as usize) as u16;
                f.render_widget(Clear, body_area);
                f.render_widget(Paragraph::new(self.lines.clone()), body_area);
            }
            return;
        }

        let max_w = self.max_line_width();
        let mut visible_w = body_area.width;
        let mut visible_h = body_area.height;
        let (mut bar_v, mut bar_h) = (false, false);
        // 任一轨道占位都可能使另一方向溢出；两轮即可稳定布局。
        // 极小区域至少保留一列/行正文，避免轨道吞掉整个内容区。
        for _ in 0..2 {
            bar_v = body_area.width > 1 && self.lines.len() > visible_h as usize;
            bar_h = body_area.height > 1 && max_w > visible_w;
            visible_w = body_area.width.saturating_sub(u16::from(bar_v));
            visible_h = body_area.height.saturating_sub(u16::from(bar_h));
        }
        self.vis_w = visible_w;
        self.vis_h = visible_h;
        self.clamp_scroll(visible_h);
        self.h_scroll = self.h_scroll.min(max_w.saturating_sub(visible_w));
        let content = Rect::new(body_area.x, body_area.y, visible_w, visible_h);
        self.v_bar_area = if bar_v {
            Rect::new(content.right(), content.y, 1, visible_h)
        } else {
            Rect::default()
        };
        self.h_bar_area = if bar_h {
            Rect::new(content.x, content.bottom(), visible_w, 1)
        } else {
            Rect::default()
        };

        // 未启用换行；只克隆当前视口，纵向偏移已由切片处理。
        let visible: Vec<_> = self.lines.iter().skip(self.scroll as usize)
            .take(visible_h as usize).cloned().collect();
        f.render_widget(
            Paragraph::new(Text::from(visible)).scroll((0, self.h_scroll)),
            content,
        );

        // 明确重置背景及文字样式，避免滑块继承新增/删除行的颜色。
        let thumb = Style::reset().patch(theme::Theme::text());
        let track = Style::reset().patch(theme::Theme::border());
        if bar_v {
            // ScrollbarState 的最后位置是 content_length - 1；这里传可滚动位置数，
            // 再单独传可视高度，保证正文到底时滑块也贴到轨道末端。
            let mut state = ScrollbarState::new(self.max_scroll_v() as usize + 1)
                .viewport_content_length(visible_h as usize)
                .position(self.scroll as usize);
            f.render_stateful_widget(
                Scrollbar::new(ScrollbarOrientation::VerticalRight)
                    .thumb_symbol("┃").track_symbol(Some("│"))
                    .begin_symbol(None).end_symbol(None)
                    .thumb_style(thumb).track_style(track),
                self.v_bar_area,
                &mut state,
            );
        }
        if bar_h {
            let mut state = ScrollbarState::new(self.max_scroll_h() as usize + 1)
                .viewport_content_length(visible_w as usize)
                .position(self.h_scroll as usize);
            f.render_stateful_widget(
                Scrollbar::new(ScrollbarOrientation::HorizontalBottom)
                    .thumb_symbol("━").track_symbol(Some("─"))
                    .begin_symbol(None).end_symbol(None)
                    .thumb_style(thumb).track_style(track),
                self.h_bar_area,
                &mut state,
            );
        }
    }
}

#[cfg(test)]
mod scrollbar_tests {
    use super::*;
    use ratatui::{backend::TestBackend, buffer::Buffer, style::Color, Terminal};

    fn draw(panel: &mut PreviewPanel, width: u16, height: u16) -> Buffer {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|f| panel.render_pane(f, f.area(), true)).unwrap().buffer.clone()
    }

    #[test]
    fn viewport_slicing_matches_full_paragraph_with_chinese_and_long_lines() {
        let raw = (0..100).map(|i| format!("+第{i:03}行：中文与很长的内容{}", "x".repeat(80)))
            .collect::<Vec<_>>().join("\n");
        let mut panel = PreviewPanel::diff("viewport", &raw);
        for (v, h, width, height) in [(0, 0, 40, 12), (30, 7, 40, 12), (99, 90, 24, 6), (70, 2, 60, 20)] {
            panel.scroll = v;
            panel.h_scroll = h;
            let actual = draw(&mut panel, width, height);
            let area = Rect::new(1, 1, panel.vis_w, panel.vis_h);
            let mut expected = Buffer::empty(Rect::new(0, 0, width, height));
            ratatui::widgets::Widget::render(
                Paragraph::new(Text::from(panel.lines.clone())).scroll((panel.scroll, panel.h_scroll)),
                area, &mut expected,
            );
            for y in area.y..area.bottom() {
                for x in area.x..area.right() {
                    assert_eq!(actual[(x, y)].symbol(), expected[(x, y)].symbol(), "at {x},{y}");
                }
            }
        }
    }

    #[test]
    fn preview_diff_limits_are_visible_without_changing_patch_lines() {
        let raw = "+original patch line\n";
        for (stdout_cut, stderr_cut, expected) in [
            (true, false, "预览已截断"),
            (false, true, "诊断输出已截断"),
        ] {
            let panel = PreviewPanel::diff_with_limits("file", raw, stdout_cut, stderr_cut);
            assert!(panel.banner.as_deref().unwrap().contains(expected));
            if stdout_cut {
                assert!(panel.banner.as_deref().unwrap().contains("返回文件树后按 E"));
            }
            assert_eq!(panel.lines[0].to_string(), "+original patch line");
        }
        assert!(PreviewPanel::diff_with_limits("file", raw, false, false).banner.is_none());
        let exact = "+line\n".repeat(MAX_LINES);
        assert!(PreviewPanel::diff_with_limits("file", &exact, false, false).banner.is_none());
        let over = format!("{exact}+one more\n");
        let panel = PreviewPanel::diff_with_limits("file", &over, false, false);
        assert_eq!(panel.lines.len(), MAX_LINES);
        assert!(panel.banner.unwrap().contains("预览已截断"));
    }

    #[test]
    fn scrollbars_have_neutral_styles_and_do_not_cover_content() {
        let raw = (0..20).map(|i| format!("+line{i:02}-{}-tail{i:02}\n", "x".repeat(30))).collect::<String>();
        let mut panel = PreviewPanel::diff("changes.txt", &raw);
        let buffer = draw(&mut panel, 24, 10);
        for y in panel.v_bar_area.y..panel.v_bar_area.bottom() {
            let cell = &buffer[(panel.v_bar_area.x, y)];
            assert!(matches!(cell.symbol(), "│" | "┃"));
            assert!(matches!(cell.fg, Color::Gray | Color::DarkGray));
            assert_eq!(cell.bg, Color::Reset);
        }
        for x in panel.h_bar_area.x..panel.h_bar_area.right() {
            let cell = &buffer[(x, panel.h_bar_area.y)];
            assert!(matches!(cell.symbol(), "─" | "━"));
            assert!(matches!(cell.fg, Color::Gray | Color::DarkGray));
            assert_eq!(cell.bg, Color::Reset);
        }
        assert_eq!(buffer[(1, 1)].symbol(), "+");
        assert_eq!(buffer[(1, 1)].fg, Color::Green);
        assert_eq!(buffer[(panel.v_bar_area.x, panel.v_bar_area.y)].symbol(), "┃");
        assert_eq!(buffer[(panel.h_bar_area.x, panel.h_bar_area.y)].symbol(), "━");
        let corner = (panel.v_bar_area.x, panel.h_bar_area.y);
        assert_eq!(buffer[corner].symbol(), " ");
        assert_eq!(panel.hit_bar(corner.0, corner.1), None);

        panel.scroll_down(u16::MAX);
        panel.scroll_right(u16::MAX);
        let buffer = draw(&mut panel, 24, 10);
        assert_eq!(buffer[(panel.v_bar_area.x, panel.v_bar_area.bottom() - 1)].symbol(), "┃");
        assert_eq!(buffer[(panel.h_bar_area.right() - 1, panel.h_bar_area.y)].symbol(), "━");
        let last_row = (1..panel.v_bar_area.x).map(|x| buffer[(x, panel.v_bar_area.bottom() - 1)].symbol()).collect::<String>();
        assert!(last_row.ends_with("tail19"), "最后一行和长行末尾应完整可见：{last_row}");
    }

    #[test]
    fn drag_tracks_reach_both_ends_and_exclude_text() {
        let mut panel = PreviewPanel::diff("long", &format!("+{}\n", "x".repeat(50)).repeat(30));
        draw(&mut panel, 24, 10);
        let (v, h) = (panel.v_bar_area, panel.h_bar_area);
        assert_eq!(panel.hit_bar(v.x, v.y), Some(BarHit::Vertical));
        assert_eq!(panel.hit_bar(h.x, h.y), Some(BarHit::Horizontal));
        assert_eq!(panel.hit_bar(1, 1), None);
        panel.drag_v(v.bottom() - 1);
        panel.drag_h(h.right() - 1);
        assert_eq!(panel.scroll, panel.max_scroll_v());
        assert_eq!(panel.h_scroll, panel.max_scroll_h());
        panel.drag_v(v.y);
        panel.drag_h(h.x);
        assert_eq!((panel.scroll, panel.h_scroll), (0, 0));
    }

    #[test]
    fn gutters_account_for_each_other_and_use_display_width() {
        let mut fit = PreviewPanel::diff("fit", "+中文");
        draw(&mut fit, 7, 4);
        assert_eq!(fit.vis_w, 5);
        assert_eq!(fit.v_bar_area, Rect::default());
        assert_eq!(fit.h_bar_area, Rect::default());
        for (lines, width) in [(4, 11), (5, 10)] {
            let mut panel = PreviewPanel::diff("edge", &format!("{}\n", "x".repeat(width)).repeat(lines));
            draw(&mut panel, 12, 6);
            assert_eq!((panel.vis_w, panel.vis_h), (9, 3));
            assert_eq!(panel.v_bar_area.height, 3);
            assert_eq!(panel.h_bar_area.width, 9);
        }
        for width in 0..5 {
            for height in 0..5 {
                draw(&mut fit, width, height);
            }
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

/// 保留文本读取入口；图片内容由图片面板绘制，其他类型遵守相同的大小和归档限制。
pub fn read_file(path: &std::path::Path) -> std::io::Result<String> {
    use super::file_preview::{self, FileContent};
    Ok(match file_preview::load(path)? {
        FileContent::Text(text) | FileContent::Document(text) => text,
        FileContent::Disabled(reason) => reason,
        FileContent::Image { .. } => "图片文件，请使用图片预览面板查看".into(),
    })
}
