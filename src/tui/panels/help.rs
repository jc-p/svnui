//! 完整帮助（`?` 或 `F1`）。
//!
//! 底栏只放得下 4~5 个最常用的键，剩下的都在这里。
//! 按分组排列，比塞在一行里好找得多。

use ratatui::{
    layout::{Alignment, Rect},
    text::{Line, Span},
    widgets::{Borders, Clear, Paragraph},
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
            ("Tab / Shift+Tab", "切换文件树与内嵌预览焦点（高亮边框）"),
            ("↑↓ / j k", "文件树焦点：选择文件；预览焦点：滚动内容"),
            ("PgUp / PgDn", "预览焦点：按可见高度翻页"),
            ("Home / End", "预览焦点：回顶部 / 到底部（也可 g / G）"),
            ("Esc", "预览焦点：返回文件树，保留过滤及滚动位置"),
            ("J / K", "滚动内嵌预览（大写：j/k 已被光标占用）"),
            ("Ctrl+d / Ctrl+u", "内嵌预览纵向翻页"),
            ("拖拽滚动条", "直接拖预览区滚动条（M 键开关鼠标）"),
            ("← / →", "横向慢移（看清长行末尾）"),
            ("< / >", "横向翻页（约 3/4 屏宽）"),
            ("0", "回到最左（列 0）"),
            ("Enter", "打开：目录=展开折叠，文件=全屏看内容"),
            ("图片预览", "PNG/JPEG/WebP/GIF 首帧/BMP/ICO，自动适应窗口；Enter 全屏"),
            ("预览限制", "压缩包及超过 2 MiB 的文件关闭内容预览，图片和办公文档同样受限"),
            ("Space", "勾选/取消当前可提交文件"),
            ("E", "用外部编辑器打开（$VISUAL / $EDITOR / vim）"),
            ("o", "用系统默认应用打开当前文件（图片全屏内也可用）"),
        ],
    },
    Group {
        title: "查看",
        keys: &[
            ("T", "切换「只看变更」和「完整文件树」"),
            ("S", "排序循环：按状态 → 按路径 → 按文件名"),
            ("L", "提交历史（右侧显示每个版本改了什么）"),
            ("Ctrl+B", "svn blame：这一行最后是谁改的、在哪一版改的"),
            ("I", "svn info：工作副本根 / URL / 版本 / 仓库 UUID"),
            ("H", "工作副本体检：冲突、锁、缺失"),
            ("V", "远端仓库浏览：直接看服务器上的目录树（可复制 URL / 就地检出）"),
            ("B", "分支工作流：F1 创建分支 / F2 标签 / F3 switch / F4 合并预演"),
            ("R / F5", "立即扫描并保留位置（空闲主界面也会自动刷新）"),
            ("Esc", "清空文件过滤；无过滤时退出"),
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
            ("a", "添加当前未纳管文件；选中目录时列出可裁剪清单（Space 剔除不要的）"),
            ("A", "批量添加：列出全部未纳管项，可裁剪；有搜索时默认预选匹配项"),
            ("r", "丢弃本地改动：勾选项优先，否则当前文件/目录；先列范围及后果，Y 执行、Enter 取消"),
            ("D", "svn delete：从版本控制删除（目录是递归的；可切「保留本地文件」开关）"),
            ("u", "svn update：先查远端更新，有冲突风险会预警"),
            ("U", "svn update -r 版本号：更新到指定版本（可填 1234 或 1200:1234）"),
            ("X", "冲突解决：三路对比，选一份直接 resolve"),
            ("Ctrl+L", "svn lock / unlock：默认加锁（期间别人提交会被拒），开关可切为解锁"),
            ("Ctrl+K", "svn cleanup：解锁 / 清除中断的操作；可选同时删除未纳管或忽略的文件"),
        ],
    },
    Group {
        title: "提交",
        keys: &[
            ("Space", "在文件树上勾选/取消提交或撤销的文件"),
            ("Ctrl+A", "先勾当前可见项；再按一次，把折叠/搜索过滤外的也勾上"),
            ("i", "反选当前可见项（「除了这几个都要」不用先全选再挨个取消）"),
            ("Ctrl+R", "清空勾选"),
            ("C", "打开提交面板（只写说明，文件清单在上面）"),
            ("Ctrl+G", "生成提交信息（配了 llm 走模型，否则本地规则；再按轮换候选）"),
            ("Ctrl+S", "提交"),
            ("Esc", "退出提交框，写的内容会保留，按 C 回来继续"),
        ],
    },
    Group {
        title: "其他",
        keys: &[
            ("?", "显示/隐藏本帮助"),
            ("F4", "查看完整提示：自动换行；↑↓/PgUp/PgDn 滚动，Home/End 首尾，y 复制全文，Esc 关闭"),
            ("", ""),
            ("子面板内（只在对应面板里生效）：", ""),
            ("X 冲突面板", "Enter 保留现状 / m 用我的 / t 用服务器的 / 1 2 3 仅切换预览"),
            ("V 仓库面板", "Enter 进目录 / ← 上级 / o 检出 / Y 复制路径 / Shift+Y 复制本目录"),
            ("L 历史面板", "Enter 看该版本改了什么 / d 设对比基准 / D 与基准做区间对比 / R 回退到选中版本 / / 搜索"),
            ("确认框内", "Tab 切开关 / Space 切开关或裁剪项 / 版本号可直接键入 / Backspace 删除"),
            ("确认框清单", "j/k 移动光标 / Space 剔除或加回 / Ctrl+A 全选 / Ctrl+R 清空；Y 只执行选中的"),
            ("M", "开关鼠标（开=可拖滚动条；关=可用鼠标选中文本复制）"),
            ("Q / Esc", "退出"),
            ("", ""),
            ("命令行：", ""),
            ("svnui checkout", "检出工作副本（TUI 里不在副本时会自动进）"),
            ("svnui login", "登录并缓存凭据"),
            ("svnui auth", "列出已缓存的凭据"),
            ("svnui llm", "LLM 配置自检（--test 实发请求验证连通性）"),
            ("svnui config", "配置文件状态（--init 生成、--path 打路径）"),
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

        let block = theme::block()
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
