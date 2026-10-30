//! 基于 themekit 的统一主题入口；面板只使用 SVN 和界面语义样式。

use std::sync::OnceLock;

use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph, Scrollbar, ScrollbarOrientation};
use ratatui::Frame;
use ratatui_themekit::{Theme as KitTheme, ThemeExt};

// 一次启动固定一套主题，后台预览线程与 UI 线程共享，避免预览串色。
static ACTIVE: OnceLock<ThemeState> = OnceLock::new();

struct ThemeState {
    palette: Box<dyn KitTheme>,
    dim: Style,
}

impl ThemeState {
    fn new(palette: Box<dyn KitTheme>) -> Self {
        // 次要信息也必须在背景和选中底色上可读；对比度计算只做一次。
        let dim = Style::default().fg(readable_secondary(palette.as_ref()));
        Self { palette, dim }
    }
}

fn resolve(id: &str, no_color: bool) -> (Box<dyn KitTheme>, Option<String>) {
    if no_color || id == "no-color" {
        return (Box::new(ratatui_themekit::NO_COLOR), None);
    }
    let known = ratatui_themekit::available_theme_ids().contains(&id);
    let notice = (!known).then(|| format!("未知主题 {id:?}，已使用 catppuccin；请检查 [ui].theme"));
    let theme = ratatui_themekit::BUILTIN_THEMES
        .iter()
        .find(|t| t.id == id)
        .copied()
        .unwrap_or(ratatui_themekit::CATPPUCCIN_MOCHA);
    (Box::new(theme), notice)
}

/// 只在启动时初始化；未知名称回退并通知，NO_COLOR 优先于配置。
pub fn initialize(id: &str) -> Option<String> {
    let (theme, notice) = resolve(id.trim(), ratatui_themekit::no_color_active());
    let _ = ACTIVE.set(ThemeState::new(theme));
    notice
}

fn state() -> &'static ThemeState {
    ACTIVE.get_or_init(|| {
        let cfg = crate::config::Config::load();
        ThemeState::new(resolve(cfg.ui.theme.trim(), ratatui_themekit::no_color_active()).0)
    })
}

fn palette() -> &'static dyn KitTheme {
    state().palette.as_ref()
}

/// Clear 仍负责清除旧内容和图片标记，随后恢复主题背景，防止弹框露出终端底色。
pub fn clear(f: &mut Frame, area: Rect) {
    f.render_widget(Clear, area);
    f.render_widget(Block::default().style(Theme::base()), area);
}

/// 圆角、背景、正文和边框都从同一主题获取。
pub fn block<'a>() -> Block<'a> {
    Block::default()
        .border_type(BorderType::Rounded)
        .style(Theme::base())
        .border_style(Theme::border())
        .title_style(Theme::title())
}

pub struct Theme;

impl Theme {
    pub fn base() -> Style {
        palette().style_base()
    }
    pub fn unversioned() -> Style {
        Self::dim()
    }
    pub fn added() -> Style {
        Style::default().fg(palette().diff_added())
    }
    pub fn modified() -> Style {
        Self::caution()
    }
    pub fn deleted() -> Style {
        Style::default().fg(palette().diff_removed())
    }

    /// 冲突仍通过粗体与普通删除区分，避免状态色变成唯一线索。
    pub fn conflicted() -> Style {
        Self::deleted().add_modifier(Modifier::BOLD)
    }
    pub fn structural() -> Style {
        palette().style_accent()
    }
    pub fn props() -> Style {
        Self::info()
    }
    pub fn clean() -> Style {
        Self::success()
    }
    pub fn info() -> Style {
        palette().style_info()
    }
    pub fn success() -> Style {
        palette().style_success()
    }
    pub fn caution() -> Style {
        palette().style_warning()
    }

    /// themekit 只有一档警告色；用粗体保留低、中危险等级的区别。
    pub fn warning() -> Style {
        Self::caution().add_modifier(Modifier::BOLD)
    }
    pub fn danger() -> Style {
        let style = palette().style_error();
        if palette().id() == "no-color" {
            style.add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
        } else {
            style
        }
    }
    pub fn danger_bold() -> Style {
        Self::danger().add_modifier(Modifier::BOLD)
    }
    pub fn hotkey() -> Style {
        palette().style_accent().add_modifier(Modifier::BOLD)
    }
    pub fn conflict_theirs() -> Style {
        Self::success().add_modifier(Modifier::BOLD)
    }
    pub fn dir() -> Style {
        palette().style_accent()
    }
    pub fn body() -> Style {
        palette().style_bright()
    }
    pub fn border() -> Style {
        palette().style_border()
    }
    pub fn border_active() -> Style {
        palette().style_accent().add_modifier(Modifier::BOLD)
    }
    pub fn title() -> Style {
        Self::body().add_modifier(Modifier::BOLD)
    }

    /// 高亮只覆盖背景，保留行内的 SVN 状态色；无色模式仍能看见光标。
    pub fn selected() -> Style {
        if palette().id() == "no-color" {
            Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)
        } else {
            Style::default()
                .bg(palette().surface())
                .add_modifier(Modifier::BOLD)
        }
    }

    pub fn checked() -> Style {
        Self::success().add_modifier(Modifier::BOLD)
    }
    pub fn unchecked() -> Style {
        Self::dim()
    }
    pub fn text() -> Style {
        Style::default().fg(palette().text())
    }
    pub fn dim() -> Style {
        state().dim
    }

    /// 强调色底上的文字需选高对比前景；无色模式只使用反显。
    pub fn on_accent(color: Color) -> Style {
        if palette().id() == "no-color" {
            Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)
        } else {
            Style::default()
                .fg(contrast_text(color))
                .bg(color)
                .add_modifier(Modifier::BOLD)
        }
    }

    pub fn status_bar() -> Style {
        Self::on_accent(palette().accent())
    }
    pub fn input() -> Style {
        palette().input_styles().text
    }
    pub fn author() -> Style {
        palette().style_accent()
    }
    pub fn empty_title() -> Style {
        Self::text().add_modifier(Modifier::BOLD)
    }
    pub fn input_active() -> Style {
        Self::input().add_modifier(Modifier::BOLD)
    }
}

/// RGB 按相对亮度选择黑/白文字；ANSI 主题使用终端原生黑字与明亮强调色。
fn contrast_text(color: Color) -> Color {
    match color {
        Color::Reset => Color::Reset,
        Color::Rgb(r, g, b) => {
            if luminance(r, g, b) > 0.179 {
                Color::Black
            } else {
                Color::White
            }
        }
        Color::Black | Color::DarkGray | Color::Blue | Color::Red | Color::Magenta => Color::White,
        _ => Color::Black,
    }
}

fn luminance(r: u8, g: u8, b: u8) -> f64 {
    let linear = |v: u8| {
        let v = f64::from(v) / 255.0;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b)
}

fn contrast(a: Color, b: Color) -> Option<f64> {
    let (Color::Rgb(ar, ag, ab), Color::Rgb(br, bg, bb)) = (a, b) else {
        return None;
    };
    let (a, b) = (luminance(ar, ag, ab), luminance(br, bg, bb));
    Some((a.max(b) + 0.05) / (a.min(b) + 0.05))
}

fn readable_secondary(t: &dyn KitTheme) -> Color {
    let dim = t.text_dim();
    let (Color::Rgb(r, g, b), Color::Rgb(tr, tg, tb)) = (dim, t.text()) else {
        return dim;
    };
    for step in 0..=16u16 {
        let mix = |a: u8, z: u8| ((u16::from(a) * (16 - step) + u16::from(z) * step) / 16) as u8;
        let color = Color::Rgb(mix(r, tr), mix(g, tg), mix(b, tb));
        if [t.background(), t.surface()]
            .into_iter()
            .all(|bg| contrast(color, bg).is_none_or(|ratio| ratio >= 4.5))
        {
            return color;
        }
    }
    t.text()
}

/// 统一的滚动条。
///
/// 之前 preview 自绘 `┃/│`、`━/─`，其余面板用 ratatui 默认的 `█/░` 加
/// 两端箭头 —— 同一个界面里两种滚动条，看着像两个程序拼起来的。
///
/// 统一用细条、去掉箭头：滚动条是"告诉你还有多少内容"的辅助元素，
/// 不该比正文还显眼。
pub fn scrollbar(orientation: ScrollbarOrientation) -> Scrollbar<'static> {
    let (thumb, track) = match orientation {
        ScrollbarOrientation::HorizontalBottom | ScrollbarOrientation::HorizontalTop => ("━", "─"),
        _ => ("┃", "│"),
    };
    Scrollbar::new(orientation)
        .thumb_symbol(thumb)
        .track_symbol(Some(track))
        .begin_symbol(None)
        .end_symbol(None)
        .thumb_style(Style::reset().patch(Theme::base()).patch(Theme::text()))
        .track_style(Style::reset().patch(Theme::base()).patch(Theme::border()))
}

/// 给面板标题加焦点标记。
///
/// 实心/空心圆点同时表达焦点，避免只依赖颜色；不使用需要专门字体的私有区图标。
/// 两种状态占相同列宽，并统一标题留白，切换焦点时文字不会跳动。
pub fn focus_title(title: impl std::fmt::Display, focused: bool) -> Line<'static> {
    let marker = if focused { "●" } else { "○" };
    Line::from(vec![
        Span::styled(
            format!(" {marker} "),
            if focused { Theme::hotkey() } else { Theme::dim() },
        ),
        Span::styled(
            format!("{} ", title.to_string().trim()),
            if focused { Theme::title() } else { Theme::dim() },
        ),
    ])
}

/// 居中弹框。
///
/// 四个面板各自写过一遍（`app.rs` 两个 + commit / conflict / confirm 各一个），
/// 最小宽度和最小高度的规则还不一样 —— 同一个"弹框"在 80×20 的终端上
/// 会画出四种尺寸。统一到这里，规则只有一条：百分比算出来不够就撑到下限，
/// 但**绝不超出可视区**。
pub fn centered(area: Rect, pct_w: u16, pct_h: u16, min_w: u16, min_h: u16) -> Rect {
    let w = (area.width * pct_w / 100).max(min_w).min(area.width);
    let h = (area.height * pct_h / 100).max(min_h).min(area.height);
    Rect {
        x: area.x + (area.width.saturating_sub(w)) / 2,
        y: area.y + (area.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    }
}

/// 字符串占多少显示列（中文 2 列）。
///
/// 截断一律按**列宽**算，不按字符数：把"未纳管文件"当成 5 个字符
/// 去截 4 列，中文终端上会切出半个字。
pub fn display_width(s: &str) -> usize {
    Line::from(s).width()
}

/// 按显示列宽截断，截断处补一个 `…`。
///
/// 连一列都放不下时整段丢弃 —— 留半个字符不如什么都不留。
pub fn fit(s: &str, width: usize) -> String {
    if display_width(s) <= width {
        return s.to_string();
    }
    let room = match width.checked_sub(1) {
        Some(0) | None => return String::new(),
        Some(r) => r,
    };
    let mut out = String::new();
    let mut used = 0;
    for ch in s.chars() {
        let w = display_width(&ch.to_string());
        if used + w > room {
            break;
        }
        out.push(ch);
        used += w;
    }
    out.push('…');
    out
}

/// 空状态：居中的标题 + 若干提示行。
///
/// 空**不是**错误 —— 用暗淡色而不是红色，否则"这个目录是空的"
/// 看起来跟"读取失败"一样。各面板复用同一个，别一家写一个说法。
pub fn empty_state<'a>(title: &'a str, hints: &[&'a str]) -> Paragraph<'a> {
    let mut lines = vec![
        Line::from(""),
        Line::from(Span::styled(format!(" {title} "), Theme::empty_title())),
    ];
    for hint in hints {
        lines.push(Line::from(Span::styled(format!(" {hint} "), Theme::dim())));
    }
    Paragraph::new(lines).alignment(Alignment::Center)
}

/// 按状态符号取颜色。
///
/// 输入是 porcelain 的第一列字符（或冒泡后的最严重符号）。
pub fn for_sign(sign: char) -> Style {
    match sign {
        '?' => Theme::unversioned(),
        'A' => Theme::added(),
        'M' => Theme::modified(),
        'D' | '!' => Theme::deleted(),
        'C' | 'T' => Theme::conflicted(),
        'R' | '~' => Theme::structural(),
        'I' | 'X' | '_' => Theme::props(),
        _ => Style::default(),
    }
}

/// 严重度色阶上相邻两格的对比。
///
/// 空状态、提示条、确认面板都要靠"黄和红不一样"来传达严重程度，
/// 两格撞色等于这把尺子废了一半。
#[cfg(test)]
mod severity_tests {
    use super::*;
    use crate::policy::Danger;
    use crate::tui::panels::confirm::danger_color;
    use crate::tui::panels::notice::Level;

    fn fg(style: Style) -> Color {
        style.fg.expect("严重度色阶每一格都必须有前景色")
    }

    #[test]
    fn every_rung_of_the_scale_is_a_real_color() {
        for s in [
            Theme::info(),
            Theme::success(),
            Theme::caution(),
            Theme::warning(),
            Theme::danger(),
            Theme::danger_bold(),
        ] {
            assert!(s.fg.is_some());
        }
    }

    #[test]
    fn caution_warning_and_danger_are_distinguishable_without_color() {
        let (c, w, d) = (Theme::caution(), Theme::warning(), Theme::danger());
        assert_ne!(c, w, "注意和警戒撞色，等于少了一档");
        assert_ne!(w, d, "警戒和危险撞色，最要紧的一档看不出来");
        assert_ne!(c, d);
    }

    /// 同一把尺子：提示条的错误级、确认面板的危险级必须落在同一个红上。
    ///
    /// 这两处如果各写一份，值也许碰巧一样，但没有任何东西能保证
    /// 以后改了一处另一处会跟着变。
    #[test]
    fn panels_share_one_severity_ruler() {
        assert_eq!(fg(Level::Error.style()), fg(Theme::danger()));
        assert_eq!(fg(Level::Warning.style()), fg(Theme::caution()));
        assert_eq!(danger_color(Danger::High), fg(Theme::danger()));
        assert_eq!(danger_color(Danger::Medium), fg(Theme::warning()));
        assert_eq!(danger_color(Danger::Low), fg(Theme::caution()));
    }

    /// 强调版只加粗，不换色 —— 否则"危险"会有两个红。
    #[test]
    fn danger_bold_only_adds_emphasis() {
        assert_eq!(fg(Theme::danger_bold()), fg(Theme::danger()));
        assert!(Theme::danger_bold().add_modifier.contains(Modifier::BOLD));
    }

    /// 冲突两侧必须自带身份：三路对比翻页后靠位置记哪边是谁会搞反。
    #[test]
    fn the_two_conflict_sides_are_distinguishable() {
        assert_ne!(Theme::danger_bold(), Theme::conflict_theirs());
    }
}

#[cfg(test)]
mod adaptation_tests {
    use super::*;
    use crate::tui::panels::{
        checkout::CheckoutPanel,
        confirm::{ConfirmAction, ConfirmPanel},
        notice::{Level, NoticePanel},
        preview::PreviewPanel,
        projects::ProjectsPanel,
    };
    use ratatui::{
        backend::TestBackend,
        layout::{Constraint, Layout},
        Terminal,
    };

    #[test]
    fn all_builtin_palettes_keep_secondary_text_readable() {
        for t in ratatui_themekit::BUILTIN_THEMES {
            let dim = readable_secondary(t);
            for bg in [t.background(), t.surface()] {
                if let Some(ratio) = contrast(dim, bg) {
                    assert!(ratio >= 4.5, "{} 的次要文字对比度不足：{ratio}", t.id());
                }
            }
        }
        assert_eq!(contrast_text(Color::Rgb(240, 230, 210)), Color::Black);
        assert_eq!(contrast_text(Color::Rgb(20, 30, 40)), Color::White);
    }

    #[test]
    fn all_theme_names_resolve_and_no_color_has_priority() {
        for id in ratatui_themekit::available_theme_ids() {
            let (t, notice) = resolve(id, false);
            assert_eq!(t.id(), id);
            assert!(notice.is_none());
            assert_eq!(resolve(id, true).0.id(), "no-color");
        }
        assert_eq!(resolve("no-color", false).0.id(), "no-color");
        let (t, notice) = resolve("typo", false);
        assert_eq!(t.id(), "catppuccin");
        assert!(notice.unwrap().contains("typo"));
    }

    #[test]
    fn configured_themes_render_panels_and_share_the_palette_with_workers() {
        const TEST: &str = "tui::theme::adaptation_tests::configured_themes_render_panels_and_share_the_palette_with_workers";
        if std::env::var_os("SVNUI_THEME_TEST_CHILD").is_none() {
            // 每个主题独立进程，保持主题只在启动初始化、测试间不修改全局环境。
            for (id, no_color) in [
                ("catppuccin", false),
                ("nord", false),
                ("terminal", false),
                ("no-color", false),
                ("tokyo-night", false),
                ("typo", false),
                ("nord", true),
            ] {
                let tmp = tempfile::tempdir().unwrap();
                let config = tmp.path().join("config.toml");
                std::fs::write(&config, format!("[ui]\ntheme = {id:?}\n")).unwrap();
                let mut cmd = std::process::Command::new(std::env::current_exe().unwrap());
                cmd.args(["--exact", TEST, "--nocapture"])
                    .env("SVNUI_THEME_TEST_CHILD", id)
                    .env("SVNUI_CONFIG", config)
                    .env("SVNR_CACHE_DIR", tmp.path().join("cache"))
                    .env_remove("NO_COLOR");
                if no_color {
                    cmd.env("NO_COLOR", "1");
                }
                let out = cmd.output().unwrap();
                assert!(
                    out.status.success(),
                    "{id} / NO_COLOR={no_color}: {}\n{}",
                    String::from_utf8_lossy(&out.stdout),
                    String::from_utf8_lossy(&out.stderr)
                );
            }
            return;
        }
        let id = std::env::var("SVNUI_THEME_TEST_CHILD").unwrap();
        let notice = initialize(&crate::config::Config::load().ui.theme);
        let expected = if ratatui_themekit::no_color_active() || id == "no-color" {
            "no-color"
        } else if id == "typo" {
            "catppuccin"
        } else {
            &id
        };
        assert_eq!(palette().id(), expected);
        assert_eq!(notice.is_some(), id == "typo");
        assert_ne!(Theme::caution(), Theme::warning());
        assert_ne!(Theme::warning(), Theme::danger());
        assert_ne!(Theme::danger_bold(), Theme::conflict_theirs());
        let worker_style = std::thread::spawn(Theme::added).join().unwrap();
        assert_eq!(worker_style, Theme::added());

        for (width, height) in [(110, 24), (44, 12)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            let mut projects = ProjectsPanel::new(
                vec![crate::projects::Project {
                    path: "/workspace/svnui".into(),
                    name: "svnui · 当前项目".into(),
                    source: crate::projects::Source::Registered,
                    state: crate::projects::State::Ready,
                }],
                Some("/workspace/svnui".into()),
            );
            let frame = terminal.draw(|f| projects.render(f, f.area())).unwrap();
            // Ratatui 将双宽字符的占位格重置；真正显示的字形仍覆盖两列。
            let unexpected: Vec<_> = frame
                .buffer
                .content
                .iter()
                .enumerate()
                .filter(|(i, c)| {
                    let continuation = i % usize::from(width) > 0
                        && Line::from(frame.buffer.content[i - 1].symbol()).width() > 1;
                    !continuation
                        && Some(c.bg) != Theme::base().bg
                        && Some(c.bg) != Theme::selected().bg
                })
                .take(6)
                .map(|(i, c)| {
                    (
                        i % usize::from(width),
                        i / usize::from(width),
                        c.symbol(),
                        c.bg,
                    )
                })
                .collect();
            assert!(unexpected.is_empty(), "主题背景遗漏：{unexpected:?}");
            let text: String = frame.buffer.content.iter().flat_map(|c| c.symbol().chars())
                .filter(|c| !c.is_whitespace()).collect();
            assert!(text.contains("svnui"));
            assert!(text.contains("当前"));
            assert!(text.contains("●项目"));
            assert!(text.contains("◆"));
            if expected == "no-color" {
                assert!(frame
                    .buffer
                    .content
                    .iter()
                    .all(|c| c.fg == Color::Reset && c.bg == Color::Reset));
                assert!(frame
                    .buffer
                    .content
                    .iter()
                    .any(|c| c.modifier.contains(Modifier::REVERSED)));
            }
            let checkout = CheckoutPanel::new("/workspace/new-project".into());
            terminal.draw(|f| checkout.render(f, f.area())).unwrap();
            let mut confirm = ConfirmPanel::new(
                "还原修改",
                "此操作会丢弃本地修改",
                vec!["src/main.rs".into()],
                crate::policy::Op::Revert,
                ConfirmAction::Revert(Vec::new()),
            )
            .with_pick()
            .with_toggles(vec![crate::tui::panels::confirm::Toggle::new(
                "保留本地文件",
                true,
            )]);
            terminal.draw(|f| confirm.render(f, f.area())).unwrap();
            let mut notice =
                NoticePanel::with_level("读取失败：请检查工作副本权限".into(), Level::Error);
            terminal.draw(|f| notice.render(f, f.area())).unwrap();
            terminal.draw(|f| notice.render_toast(f, f.area(), "F4")).unwrap();
            let mut preview = PreviewPanel::diff("src/main.rs", "+新增内容\n-删除内容\n 上下文\n");
            terminal
                .draw(|f| {
                    let [head, body] =
                        Layout::vertical([Constraint::Length(1), Constraint::Min(0)])
                            .areas(f.area());
                    f.render_widget(
                        Paragraph::new(" SVN · trunk@r128 ").style(Theme::status_bar()),
                        head,
                    );
                    preview.render_pane(f, body, true);
                })
                .unwrap();
        }
    }
}
