//! 配色 —— 界面里所有颜色的唯一出处。
//!
//! 面板不许自己写 `Color::`：同一个"红"如果确认框里读作"危险"、
//! 提示条里读作"普通失败"，用户就得靠位置猜严重程度。
//! 语义在这里定义一次，各处按语义取用。

use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph, Scrollbar, ScrollbarOrientation};

/// 所有面板共用圆角边框；标题、颜色和边框范围仍由各面板决定。
pub fn block<'a>() -> Block<'a> {
    Block::default().border_type(BorderType::Rounded)
}

/// 状态语义色。
///
/// 命名沿用 SVN porcelain 的符号，方便对照。
pub struct Theme;

impl Theme {
    /// `?` 未版本化 —— 暗淡，它不是"问题"，只是还没纳入管理。
    pub fn unversioned() -> Style {
        Style::default().fg(Color::DarkGray)
    }

    /// `A` 新增 —— 绿。
    pub fn added() -> Style {
        Style::default().fg(Color::Green)
    }

    /// `M` 修改 —— 黄。
    pub fn modified() -> Style {
        Style::default().fg(Color::Yellow)
    }

    /// `D` / `!` 删除或缺失 —— 红。
    pub fn deleted() -> Style {
        Style::default().fg(Color::Red)
    }

    /// `C` / `T` 冲突 —— 红字加粗，这是唯一需要立刻处理的。
    ///
    /// 不铺红底：底色会把整行（连同选中态）一起盖掉，深色主题下
    /// 连续几行红块时既读不出字、也看不出光标停在哪一行。
    pub fn conflicted() -> Style {
        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
    }

    /// `R` 替换、`~` 阻碍等 —— 品红，表示"结构变了"。
    pub fn structural() -> Style {
        Style::default().fg(Color::Magenta)
    }

    /// `I` / `X` / `_M` 属性类 —— 青。
    pub fn props() -> Style {
        Style::default().fg(Color::Cyan)
    }

    /// 干净 / 无改动。
    pub fn clean() -> Style {
        Style::default().fg(Color::Green)
    }

    // ── 严重度色阶 ──
    //
    // 提示条（notice）、确认面板（confirm 的危险等级）、读取失败面板
    // （app / repo）三处共用这一套：黄→亮黄→亮红是同一把尺子上的三个刻度。
    // 分开各写一遍的话，值也许碰巧一样，但没有任何一处能说明
    // "确认框的这个红 = 提示条的那个红"。

    /// 信息 —— 青。
    pub fn info() -> Style {
        Style::default().fg(Color::Cyan)
    }

    /// 成功 —— 绿。
    pub fn success() -> Style {
        Style::default().fg(Color::Green)
    }

    /// 注意 —— 黄：可以继续，但得知道刚刚发生了什么。
    ///
    /// 对应确认面板的 `Danger::Low`、提示条的警告级。
    pub fn caution() -> Style {
        Style::default().fg(Color::Yellow)
    }

    /// 警戒 —— 亮黄：会动到别人的东西或丢掉本地内容。
    ///
    /// 对应确认面板的 `Danger::Medium`。比 `caution()` 亮一档，
    /// 让人在按下之前多停半秒。
    pub fn warning() -> Style {
        Style::default().fg(Color::LightYellow)
    }

    /// 危险 / 失败 —— 亮红：这一步不可逆，或者已经出错了。
    ///
    /// 既用于"还没发生的危险"（确认面板 `Danger::High`），
    /// 也用于"已经发生的失败"（读取日志/目录失败）——
    /// 对用户来说两者的处置方式一样：停下来看清楚。
    pub fn danger() -> Style {
        Style::default().fg(Color::LightRed)
    }

    /// 危险强调 —— 亮红加粗。
    ///
    /// 只给标题、徽标这类短文本：整段正文加粗会糊成一片，
    /// 但标题不加粗的话，危险面板和普通面板扫一眼分不出来。
    pub fn danger_bold() -> Style {
        Style::default().fg(Color::LightRed).add_modifier(Modifier::BOLD)
    }

    /// 键位提示 / 当前页签 —— 黄加粗。
    ///
    /// 用 `caution()` 的黄：它和"注意"是同一个意思（"看这里"），
    /// 加粗只是为了在暗色说明文字里跳出来。
    pub fn hotkey() -> Style {
        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
    }

    /// 冲突里"服务器的"那一侧（`>>>>>>>`）—— 亮绿加粗。
    ///
    /// 和"我的"（`danger_bold()`）成对：三路对比时两侧必须一眼可分，
    /// 靠位置记容易在翻页后搞反。
    pub fn conflict_theirs() -> Style {
        Style::default()
            .fg(Color::LightGreen)
            .add_modifier(Modifier::BOLD)
    }

    /// 目录标记 `▸` —— 亮蓝。
    pub fn dir() -> Style {
        Style::default().fg(Color::LightBlue)
    }

    /// 正文白字 —— 必须看得清、但又不是标题的地方。
    ///
    /// 和 `text()`（灰）的区别：`text()` 适合大段上下文行，
    /// 但铺在 `selected()` 的深灰底上会糊掉（深灰叠深灰），
    /// 列表里那些不能被高亮吃掉的字要用白。
    pub fn body() -> Style {
        Style::default().fg(Color::White)
    }

    /// 边框（普通面板）。
    pub fn border() -> Style {
        Style::default().fg(Color::DarkGray)
    }

    /// 边框（当前聚焦面板） —— 用亮色区分焦点。
    pub fn border_active() -> Style {
        Style::default().fg(Color::Cyan)
    }

    /// 标题。
    pub fn title() -> Style {
        Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
    }

    /// 选中行（列表光标）。
    pub fn selected() -> Style {
        Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD)
    }

    /// 已勾选（提交面板里的 [x]）。
    pub fn checked() -> Style {
        Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)
    }

    /// 未勾选。
    pub fn unchecked() -> Style {
        Style::default().fg(Color::DarkGray)
    }

    /// 正文文本（diff 的上下文行等）。
    ///
    /// 和 `status_bar()` 区分开：那个是"黑字青底"的条状样式，
    /// 用在正文上会变成一整片蓝。
    pub fn text() -> Style {
        Style::default().fg(Color::Gray)
    }

    /// 帮助行 / 次要信息。
    pub fn dim() -> Style {
        Style::default().fg(Color::DarkGray)
    }

    /// 顶部状态栏。
    pub fn status_bar() -> Style {
        Style::default().fg(Color::Black).bg(Color::Cyan)
    }

    /// 输入框里的正文 —— **白字**。
    ///
    /// 之前输入框用的是 `status_bar()`（黑字青底）：那是给顶部条状区域
    /// 设计的，铺在输入框里是一整片青底，字被底色压住反而看不清。
    /// 输入框就是要白字，"当前焦点"交给 Block 的边框色去表达。
    pub fn input() -> Style {
        Style::default().fg(Color::White)
    }

    /// 提交作者。
    ///
    /// 以前直接借 `modified()`（"M 修改"的黄）显示作者名 —— 语义串台：
    /// 黄色在状态列里代表"文件被改过"，挪到作者列就成了"这个人有改动"。
    pub fn author() -> Style {
        Style::default().fg(Color::LightBlue)
    }
    /// 空状态：标题与说明。
    ///
    /// 空不是错误 —— 用暗淡色而不是红色，避免"没有数据"看起来像"出事了"。
    pub fn empty_title() -> Style {
        Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD)
    }

    /// 输入框正文（聚焦中）—— 白字加粗，和未聚焦的框拉开差别。
    ///
    /// 只有边框变色不够：四个框排在一起时，用户要盯着边框找焦点。
    /// 字本身加粗，余光就能定位。
    pub fn input_active() -> Style {
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD)
    }
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
        .thumb_style(Style::reset().patch(Theme::text()))
        .track_style(Style::reset().patch(Theme::border()))
}

/// 给面板标题加焦点标记。
///
/// 光靠边框颜色区分焦点不够：四个框并排时，用户得挨个盯着边框找
/// 哪条线亮一点。在标题前面加一条竖杠，扫一眼就知道键会落在哪。
pub fn focus_title(title: impl std::fmt::Display, focused: bool) -> String {
    if focused {
        format!("▌{title}")
    } else {
        title.to_string()
    }
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
    fn caution_warning_and_danger_are_never_the_same_color() {
        let (c, w, d) = (
            fg(Theme::caution()),
            fg(Theme::warning()),
            fg(Theme::danger()),
        );
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
        assert_ne!(fg(Theme::danger_bold()), fg(Theme::conflict_theirs()));
    }
}
