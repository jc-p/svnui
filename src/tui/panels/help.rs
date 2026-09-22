//! 完整帮助（`?` 或 `F1`）。
//!
//! 底栏只放得下 4~5 个最常用的键，剩下的都在这里。
//! 按分组排列，比塞在一行里好找得多。

use ratatui::{
    layout::{Alignment, Rect},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

use crate::tui::theme;

/// 一组按键说明。
struct Group {
    title: &'static str,
    keys: &'static [(&'static str, &'static str)],
}

const GROUPS: &[Group] = &[
    Group {
        title: "导航",
        keys: &[
            ("↑↓ / j k", "上下移动光标"),
            ("J / K", "滚动右侧预览（大写：j/k 已被光标占用）"),
            ("Ctrl+d / Ctrl+u", "右侧预览纵向翻页"),
            ("拖拽滚动条", "直接拖右侧/底部滚动条（M 键开关鼠标）"),
            ("← / →", "横向慢移（看清长行末尾）"),
            ("< / >", "横向翻页（约 3/4 屏宽）"),
            ("0", "回到最左"),
            ("0", "回到最左（列 0）"),
            ("Enter / Space", "打开：目录=展开折叠，文件=全屏看内容"),
            ("E", "用外部编辑器打开（$VISUAL / $EDITOR / vim）"),
        ],
    },
    Group {
        title: "查看",
        keys: &[
            ("T", "切换「只看变更」和「完整文件树」"),
            ("S", "排序循环：按状态 → 按路径 → 按文件名"),
            ("L", "提交历史（右侧显示每个版本改了什么）"),
            ("H", "工作副本体检：冲突、锁、缺失"),
            ("V", "远端仓库浏览：直接看服务器上的目录树（可复制 URL / 就地检出）"),
            ("F5", "重新扫描（文件在外部被改动时用）"),
        ],
    },
    Group {
        title: "搜索",
        keys: &[
            ("/", "搜索（主界面搜路径，历史里搜 message/作者/版本号）"),
            ("Enter", "确认搜索，保留过滤"),
            ("Esc", "取消搜索，清除过滤"),
        ],
    },
    Group {
        title: "改动",
        keys: &[
            ("A", "svn add：把未版本化的文件纳入管理"),
            ("r", "svn revert：先干跑列出影响，二次确认后才执行（小写，别和大写搞混）"),
            ("D", "svn delete：从版本控制删除（目录是递归的）"),
            ("U", "svn update：先查远端更新，有冲突风险会预警"),
            ("X", "冲突解决：三路对比，选一份直接 resolve"),
        ],
    },
    Group {
        title: "提交",
        keys: &[
            ("Space", "在文件树上勾选/取消要提交的文件"),
            ("Ctrl+A", "全选所有可提交的文件"),
            ("Ctrl+R", "清空勾选"),
            ("C", "打开提交面板（只写说明，文件清单在上面）"),
            ("Enter", "提交框里是**换行**，不是提交"),
            ("Ctrl+S / F2", "提交"),
            ("Esc", "退出提交框，写的内容会保留，按 C 回来继续"),
        ],
    },
    Group {
        title: "其他",
        keys: &[
            ("?", "显示/隐藏本帮助"),
            ("M", "开关鼠标（开=可拖滚动条；关=可用鼠标选中文本复制）"),
            ("Q / Esc", "退出"),
            ("", ""),
            ("命令行：", ""),
            ("svnui checkout", "检出工作副本（TUI 里不在副本时会自动进）"),
            ("svnui login", "登录并缓存凭据"),
            ("svnui auth", "列出已缓存的凭据"),
        ],
    },
];

/// 帮助面板。只是个静态弹框，不需要保存状态。
pub struct HelpPanel;

impl HelpPanel {
    pub fn render(f: &mut Frame, area: Rect) {
        f.render_widget(Clear, area);

        let mut lines: Vec<Line> = Vec::new();

        for (gi, g) in GROUPS.iter().enumerate() {
            if gi > 0 {
                lines.push(Line::from(""));
            }
            lines.push(Line::from(Span::styled(
                format!(" {} ", g.title),
                theme::Theme::title(),
            )));

            // 按键列左对齐到统一宽度，说明列跟在后面
            let w = g.keys.iter().map(|(k, _)| k.chars().count()).max().unwrap_or(0);

            for (k, desc) in g.keys {
                lines.push(Line::from(vec![
                    Span::styled(format!("   {:<width$}", k, width = w), theme::Theme::props()),
                    Span::styled("   ", theme::Theme::dim()),
                    Span::raw(*desc),
                ]));
            }
        }

        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            " 提示：底栏只显示最常用的几个键，完整列表在这里。",
            theme::Theme::dim(),
        )));

        let block = Block::default()
            .borders(Borders::ALL)
            .title(" 快捷键 ")
            .title_bottom(Line::from(" 按 ? 或 Esc 关闭 ").alignment(Alignment::Right))
            .border_style(theme::Theme::border_active());

        f.render_widget(
            Paragraph::new(ratatui::text::Text::from(lines)).block(block),
            area,
        );
    }
}
