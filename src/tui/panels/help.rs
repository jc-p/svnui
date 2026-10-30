//! 完整帮助（`?` 或 `F1`）。
//!
//! 底栏只放得下 4~5 个最常用的键，剩下的都在这里。
//! 按分组排列，比塞在一行里好找得多。

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    layout::{Alignment, Rect},
    text::{Line, Span},
    widgets::{Borders, Paragraph},
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
        title: "按键约定",
        keys: &[
            ("q / Esc", "主面板只用 q 退出；Esc 关闭当前弹层、返回文件树或清空过滤，无事可取消时不操作"),
            ("输入优先", "提交、检出和搜索里 q、? 都是文字；F1 查看帮助（分支工作流内 F1 切分支页签）"),
            (
                "大写 = 同一动作的另一档",
                "小写管当前项 / 默认档，大写管更大范围或另一种代价（a 当前 · A 全部；u 最新 · U 指定版本；d 连本地一起删 · D 只移出版本控制）",
            ),
            (
                "同一个键在哪都同一件事",
                "R 在任何界面都是刷新；r 在任何界面都是「丢弃本地改动」（历史里=回退到选中版本，冲突面板=用服务器版本）",
            ),
            (
                "破坏性动作一定有确认框",
                "不看 Shift 大小写，只看框里的清单和危险级；高危边框是红的，需要按 Y",
            ),
        ],
    },
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
            ("Space", "勾选文件；目录批选后代文件改动（含折叠项，遵守搜索）"),
            ("E", "用外部编辑器打开（$VISUAL / $EDITOR / vim）"),
            ("o", "用系统默认应用打开当前文件（图片全屏内也可用）"),
        ],
    },
    Group {
        title: "查看",
        keys: &[
            ("T", "切换「改动」和「全部文件」，保留筛选、有效勾选并尽量定位原路径"),
            ("S", "排序循环：按状态 → 按路径 → 按文件名"),
            ("L", "提交历史（右侧显示每个版本改了什么）"),
            ("F3", "只读查看本人写过的 SVN 提交说明，首次填写 SVN 用户名；无待提交文件也可使用"),
            ("Ctrl+B", "svn blame：这一行最后是谁改的、在哪一版改的"),
            ("I", "svn info：工作副本根 / URL / 版本 / 仓库 UUID"),
            ("H", "工作副本体检：冲突、锁、缺失"),
            ("Ctrl+P", "项目切换：列出这台机器上已知的工作副本，Enter 直接切过去（打开过的会自动记住）"),
            ("V", "远端仓库浏览：直接看服务器上的目录树（可复制 URL / 就地检出）"),
            ("B", "分支工作流：F1 创建分支 / F2 标签 / F3 switch / F4 合并预演"),
            ("R / F5", "刷新：主界面重新扫描并保留位置；历史面板重拉提交记录（空闲主界面也会自动刷新）"),
            ("Esc", "清空文件过滤；无过滤时停留主面板"),
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
            ("r", "丢弃本地改动：勾选项优先，否则当前文件/目录；先列范围及后果，Y 执行、Enter 取消（历史面板里＝回退到选中版本）"),
            ("d", "svn delete：连本地文件一起删（目录是递归的）—— 高危，不可撤销"),
            ("D", "svn delete：只移出版本控制、本地文件留着 —— 中危；两种都能在框里再切开关"),
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
            ("Space", "勾选文件或批选目录；[-] 表示目录内部分选中，再按补齐"),
            ("Ctrl+A", "全选当前筛选内的文件改动，包含折叠项；重复按不扩大范围"),
            ("i", "反选当前筛选内的文件改动，包含折叠项"),
            ("Ctrl+R", "清空勾选"),
            ("C", "打开提交面板（只写说明，文件清单在上面）"),
            ("F3（提交框内）", "本人提交说明：填写 SVN 用户名，精确匹配作者；Enter 插入光标处，N 更早，F2 改作者，Esc 返回草稿"),
            ("Ctrl+G", "生成提交信息（配了 llm 走模型，否则本地规则；再按轮换候选）"),
            ("Ctrl+S", "提交"),
            ("Esc", "返回文件树并保留草稿，按 C 回来继续"),
        ],
    },
    Group {
        title: "其他",
        keys: &[
            ("? / F1", "查看帮助；↑↓/j/k 滚动，PgUp/PgDn 翻页，Home/End 首尾；Esc/q/?/F1 关闭"),
            ("F4 / Shift+F4", "查看右上角提示详情（工作流用 Shift+F4）；↑↓/PgUp/PgDn 滚动，Home/End 首尾，y 复制全文，Esc 关闭"),
            ("", ""),
            ("子面板内（只在对应面板里生效）：", ""),
            ("X 冲突面板", "Enter 保留现状 / m 用我的 / t 用服务器的 / 1 2 3 仅切换预览"),
            ("V 仓库面板", "Enter 进目录 / ← 上级 / o 检出 / Y 复制路径 / Shift+Y 复制本目录"),
            ("L 历史面板", "Enter 看该版本改了什么 / d 设对比基准 / D 与基准做区间对比 / r 回退到选中版本 / R 刷新 / N 加载更早 / / 搜索"),
            ("确认框内", "Tab 切开关 / Space 切开关或裁剪项 / 版本号可直接键入 / Backspace 删除"),
            ("确认框清单", "j/k 移动光标 / Space 剔除或加回 / Ctrl+A 全选 / Ctrl+R 清空；Y 只执行选中的"),
            ("M", "开关鼠标（开=可拖滚动条；关=可用鼠标选中文本复制）"),
            ("q", "退出主面板；阅读面板内只返回上层，输入框内作为文字"),
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

/// 帮助独占按键；记录可视行数，让窄窗口也能读到末尾。
#[derive(Default)]
pub struct HelpPanel {
    scroll: usize,
    viewport: usize,
    total: usize,
}

impl HelpPanel {
    /// 返回 true 仅表示关闭帮助，不向底层继续分发此按键。
    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        let page = self.viewport.saturating_sub(1).max(1);
        match key.code {
            KeyCode::Esc | KeyCode::F(1) => return true,
            KeyCode::Char('q' | '?')
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                return true
            }
            KeyCode::Down | KeyCode::Char('j') => self.scroll = self.scroll.saturating_add(1),
            KeyCode::Up | KeyCode::Char('k') => self.scroll = self.scroll.saturating_sub(1),
            KeyCode::PageDown => self.scroll = self.scroll.saturating_add(page),
            KeyCode::PageUp => self.scroll = self.scroll.saturating_sub(page),
            KeyCode::Home | KeyCode::Char('g') => self.scroll = 0,
            KeyCode::End | KeyCode::Char('G') => self.scroll = self.max_scroll(),
            _ => {}
        }
        self.scroll = self.scroll.min(self.max_scroll());
        false
    }

    fn max_scroll(&self) -> usize {
        self.total.saturating_sub(self.viewport)
    }

    pub fn render(&mut self, f: &mut Frame, area: Rect) {
        theme::clear(f, area);
        let width = area.width.saturating_sub(2);
        if width == 0 || area.height <= 2 {
            return;
        }
        let mut lines: Vec<Line> = Vec::new();

        for (gi, g) in GROUPS.iter().enumerate() {
            if gi > 0 {
                lines.push(Line::from(""));
            }
            lines.push(Line::from(Span::styled(
                format!(" {} ", g.title),
                theme::Theme::title(),
            )));

            // 按键和说明分行，按终端显示宽度折行，避免中文及长说明被裁掉。
            for (k, desc) in g.keys {
                for row in super::notice::wrap_text(k, width) {
                    lines.push(Line::from(Span::styled(row, theme::Theme::props())));
                }
                if !desc.is_empty() {
                    for row in super::notice::wrap_text(desc, width.saturating_sub(2).max(1)) {
                        lines.push(Line::from(format!("  {row}")));
                    }
                }
            }
        }

        lines.push(Line::from(""));
        for row in
            super::notice::wrap_text("提示：底栏只显示最常用的几个键，完整列表在这里。", width)
        {
            lines.push(Line::from(Span::styled(row, theme::Theme::dim())));
        }

        self.total = lines.len();
        self.viewport = area.height.saturating_sub(2) as usize;
        self.scroll = self.scroll.min(self.max_scroll());
        let end = self.scroll.saturating_add(self.viewport).min(self.total);
        let keys = if width >= 56 {
            " ↑↓/j/k 滚动 · PgUp/PgDn 翻页 · Esc/q 关闭 "
        } else if width >= 28 {
            " ↑↓ 滚动 · Esc/q 关闭 "
        } else {
            " Esc 关闭 "
        };
        let block = theme::block()
            .borders(Borders::ALL)
            .title(format!(
                " 快捷键 · {}–{end}/{} ",
                self.scroll + 1,
                self.total
            ))
            .title_bottom(Line::from(keys).alignment(Alignment::Right))
            .border_style(theme::Theme::border_active());

        f.render_widget(
            Paragraph::new(ratatui::text::Text::from(
                lines
                    .into_iter()
                    .skip(self.scroll)
                    .take(self.viewport)
                    .collect::<Vec<_>>(),
            ))
            .block(block),
            area,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_scrolls_to_the_end_and_clamps_after_resize() {
        let mut panel = HelpPanel::default();
        for (width, height) in [(30, 10), (64, 20), (140, 40)] {
            let mut terminal =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
            terminal.draw(|f| panel.render(f, f.area())).unwrap();
            assert!(panel.total > panel.viewport);
            assert!(!panel.handle_key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE)));
            let frame = terminal.draw(|f| panel.render(f, f.area())).unwrap();
            assert_eq!(panel.scroll, panel.max_scroll());
            let text: String = frame
                .buffer
                .content
                .iter()
                .flat_map(|c| c.symbol().chars())
                .filter(|c| !c.is_whitespace())
                .collect();
            assert!(
                text.contains("完整列表在这里"),
                "{width}×{height} 必须能读到帮助末尾：{text}"
            );
            assert!(text.contains("Esc"));
            let end = panel.scroll;
            panel.handle_key(KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE));
            assert!(panel.scroll < end);
            panel.handle_key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE));
            assert_eq!(panel.scroll, 0);
        }
    }

    #[test]
    fn help_only_closes_on_explicit_close_keys() {
        let mut panel = HelpPanel::default();
        for code in [
            KeyCode::Enter,
            KeyCode::Char('C'),
            KeyCode::F(4),
            KeyCode::Down,
        ] {
            assert!(!panel.handle_key(KeyEvent::new(code, KeyModifiers::NONE)));
        }
        for code in [
            KeyCode::Esc,
            KeyCode::Char('q'),
            KeyCode::Char('?'),
            KeyCode::F(1),
        ] {
            assert!(panel.handle_key(KeyEvent::new(code, KeyModifiers::NONE)));
        }
    }

    #[test]
    fn help_documents_browse_exit_and_draft_return() {
        assert!(desc_of("查看", "Esc").contains("停留主面板"));
        assert!(desc_of("提交", "Esc").contains("保留草稿"));
        assert!(desc_of("其他", "q").contains("退出主面板"));
    }

    fn group(title: &str) -> &'static Group {
        GROUPS.iter().find(|g| g.title == title).expect(title)
    }

    fn desc_of(title: &str, key: &str) -> &'static str {
        group(title)
            .keys
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, d)| *d)
            .expect(key)
    }

    /// R 在任何界面都是刷新，不能同时是"回退到某一版"这种不可逆动作 ——
    /// 跟着主界面的手感按 R 想刷新，弹出来的是回退确认框，代价太大。
    #[test]
    fn shift_r_is_never_the_destructive_action() {
        for g in GROUPS {
            for (key, desc) in g.keys {
                assert!(
                    !desc.contains("R 回退"),
                    "R 只能是刷新，却在「{}」里变成了回退：{key} {desc}",
                    g.title
                );
            }
        }
    }

    /// d/D 必须是两档，不能还是同一句话。
    ///
    /// 完全等价的假重载比不绑大写更糟：用户按大写期待"范围更大"，
    /// 结果什么都没变，于是怀疑大写另有含义而不敢按。
    #[test]
    fn lowercase_and_uppercase_keys_are_two_different_rungs() {
        assert_ne!(
            desc_of("改动", "d"),
            desc_of("改动", "D"),
            "d/D 说明完全一样 —— 又退回到假重载了"
        );
        assert!(desc_of("改动", "d").contains("高危"));
        assert!(desc_of("改动", "D").contains("中危"));
    }
}
