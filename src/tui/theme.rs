//! 配色。
//!
//! 刻意和 同一套语义色 保持同一套语义色，
//! 这样在 两种界面 之间来回切换时不会觉得"换了套皮肤"。

use ratatui::layout::Alignment;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph};

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

    /// 仓库标识（分支名@版本号）。
    pub fn identity() -> Style {
        Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
    }

    /// 分隔/结构线（表头、分隔行）—— 比正文更暗，不抢内容。
    pub fn separator() -> Style {
        Style::default().fg(Color::DarkGray)
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
