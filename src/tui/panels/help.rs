//! 分类快捷键帮助；只负责查阅，不把任何操作传递到背后的面板。

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    text::{Line, Span},
    widgets::{
        Borders, List, ListItem, ListState, Paragraph, ScrollbarOrientation, ScrollbarState,
    },
    Frame,
};

use crate::tui::theme;

struct Group {
    title: &'static str,
    keys: &'static [(&'static str, &'static str)],
}

// 同一组只描述对应上下文；小写 r 和大写 R 的后果必须清楚区分。
const GROUPS: &[Group] = &[
    Group {
        title: "常用",
        keys: &[
            ("Space", "勾选文件；选中目录时批选其中的改动"),
            ("C", "打开提交框；Esc 返回时保留草稿"),
            ("F3", "查看本人写过的 SVN 提交说明"),
            ("Tab / Shift+Tab", "切换文件树与预览焦点"),
            ("? / F1", "打开帮助；文字输入中使用 F1"),
            ("Esc", "逐层返回或清空过滤；主面板停留，不退出"),
            ("q", "退出主面板；输入框中作为文字"),
            ("操作确认", "危险操作先核对范围；高危操作需按 Y 执行"),
        ],
    },
    Group {
        title: "导航",
        keys: &[
            ("↑↓ / j / k", "文件树中选择；预览中滚动"),
            ("Tab / Shift+Tab", "切换文件树与预览，高亮边框表示当前焦点"),
            ("Enter", "目录展开或折叠；文件全屏预览"),
            ("Esc", "从预览返回文件树，保留过滤及滚动位置"),
            ("J / K", "文件树焦点下滚动右侧预览"),
            ("PgUp / PgDn", "预览按可见高度翻页"),
            ("Ctrl+d / Ctrl+u", "内嵌预览纵向翻页"),
            ("Home / End", "预览跳到首尾；也可用 g / G"),
            ("← / →", "预览左右移动；< / > 横向翻页，0 回最左"),
            ("E / o", "用外部编辑器 / 系统默认应用打开文件"),
            ("M", "切换鼠标：开启可拖滚动条；关闭可选中文字"),
        ],
    },
    Group {
        title: "查看",
        keys: &[
            ("T", "切换改动 / 全部文件，保留筛选和有效勾选"),
            ("S", "排序：状态 → 路径 → 文件名"),
            ("L", "仓库提交历史，查看各版本的改动"),
            ("F3", "本人提交说明，无待提交文件也可查看"),
            ("Ctrl+B", "svn blame：查看每行最后修改的作者和版本"),
            ("I", "工作副本根、URL、版本与仓库 UUID"),
            ("H", "工作副本体检：冲突、锁、缺失"),
            ("Ctrl+P", "切换项目；Enter 进入选中的工作副本"),
            ("V", "浏览远端仓库，可复制 URL 或检出"),
            ("B", "分支工作流：创建分支、标签、切换与合并预演"),
            ("R / F5", "重新扫描主面板；空闲时也会自动刷新"),
            ("Esc", "清空过滤；无过滤时停留主面板"),
        ],
    },
    Group {
        title: "搜索",
        keys: &[
            ("/", "主面板搜路径；仓库历史搜说明、作者和版本号"),
            ("Enter", "确认搜索并保留过滤"),
            ("Esc", "取消搜索并清除过滤"),
            ("q / ?", "搜索中正常输入文字；F1 可临时查看帮助"),
            ("Ctrl+A", "确认搜索后，全选当前筛选内的文件改动，包含折叠项"),
            ("i", "确认搜索后，反选当前筛选内的文件改动"),
        ],
    },
    Group {
        title: "提交",
        keys: &[
            ("Space", "勾选文件或批选目录；[-] 表示部分选中"),
            ("Ctrl+A / i", "全选 / 反选当前筛选内的文件改动"),
            ("Ctrl+R", "清空勾选"),
            ("C", "打开提交框；未勾选时自动选择全部可提交改动"),
            ("Enter", "在说明中换行，不执行提交"),
            ("Ctrl+G", "生成说明；再次按下轮换候选，可继续手动编辑"),
            ("F3", "打开本人历史，将以前的说明插入光标处"),
            ("Ctrl+S / F2", "提交当前选中文件与说明"),
            ("Esc", "返回文件树并保留草稿，按 C 继续编辑"),
        ],
    },
    Group {
        title: "本人历史",
        keys: &[
            (
                "Enter（用户名）",
                "查询当前项目中该作者的提交，精确匹配且区分大小写",
            ),
            ("F2", "修改 SVN 用户名，本次会话按工作副本记住"),
            ("↑↓ / j / k", "选择一条已提交的说明"),
            ("J / K", "逐行阅读完整说明"),
            ("PgUp / PgDn", "说明翻页；Home / End 跳到首尾"),
            ("N", "查找更早记录；本批无匹配项时也可继续"),
            ("R", "重新查询"),
            ("Enter（记录）", "从提交框进入时插入光标处；主面板入口只读"),
            ("Esc / q", "返回原面板；不清空草稿，不自动提交"),
        ],
    },
    Group {
        title: "改动",
        keys: &[
            ("a", "添加当前未纳管项；目录先列清单，可用 Space 裁剪"),
            ("A", "批量添加；有搜索时默认预选匹配项"),
            ("r", "丢弃本地改动；勾选项优先，执行前确认范围"),
            ("d", "删除版本项及本地文件；高危，先核对清单"),
            ("D", "只移出版本控制，保留本地文件；中危"),
            ("u", "更新到最新版本，先检查远端及冲突风险"),
            ("U", "更新到指定版本；可填 1234 或 1200:1234"),
            ("X", "打开冲突解决面板"),
            ("Ctrl+L", "svn lock / unlock，确认框可切换"),
            ("Ctrl+K", "svn cleanup；删除未纳管或忽略项需单独选择"),
            ("R", "刷新，不执行还原；注意与小写 r 区分"),
        ],
    },
    Group {
        title: "子面板",
        keys: &[
            ("历史 · Enter", "查看该版本的改动；N 更早，R 刷新，/ 搜索"),
            ("历史 · d / D", "设对比基准 / 与基准做区间对比"),
            ("历史 · r", "回退到选中版本，执行前确认"),
            ("仓库 · Enter / ←", "进入目录 / 返回上级；o 检出"),
            ("仓库 · Y", "复制路径；Shift+Y 复制本目录"),
            (
                "冲突 · 1 / 2 / 3",
                "切换预览；Enter 保留现状，m 用我的，t 用服务器的",
            ),
            ("确认 · Tab", "切换开关或字段；Space 切换开关或裁剪项"),
            ("确认 · j / k", "清单移动；Ctrl+A 全选，Ctrl+R 清空"),
            ("工作流 · F1–F4", "创建分支 / 标签 / 切换 / 合并预演"),
            ("工作流 · Ctrl+S", "准备操作；Tab 切字段，Ctrl+U 清空字段"),
        ],
    },
    Group {
        title: "其他",
        keys: &[
            ("F4 / Shift+F4", "查看通知详情；分支工作流内用 Shift+F4"),
            ("通知 · y", "复制完整提示；↑↓ / PgUp / PgDn 阅读，Esc 关闭"),
            ("M", "开关鼠标；关闭后由终端处理文字选中与复制"),
            ("q", "退出主面板；阅读面板返回上层，输入框正常输入"),
            ("svnui probe", "检查 SVN 路径、版本、工作副本和能力"),
            ("svnui login", "登录并缓存 SVN 凭据；auth 列出已缓存凭据"),
            ("svnui config", "查看配置状态；--init 生成，--path 显示路径"),
            ("svnui llm", "检查模型配置；--test 会实际请求模型服务"),
        ],
    },
];

#[derive(Default)]
pub struct HelpPanel {
    group: usize,
    scroll: usize,
    viewport: usize,
    total: usize,
}

impl HelpPanel {
    /// 帮助消费所有输入，只有明确的关闭键返回 true。
    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        let page = self.viewport.saturating_sub(1).max(1);
        match key.code {
            KeyCode::Esc | KeyCode::F(1) => return true,
            KeyCode::Char('q' | '?')
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                return true;
            }
            KeyCode::Tab | KeyCode::Right => self.select_group((self.group + 1) % GROUPS.len()),
            KeyCode::BackTab | KeyCode::Left => {
                self.select_group((self.group + GROUPS.len() - 1) % GROUPS.len());
            }
            KeyCode::Char(ch @ '1'..='9') if key.modifiers.is_empty() => {
                let index = (ch as u8 - b'1') as usize;
                if index < GROUPS.len() {
                    self.select_group(index);
                }
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

    fn select_group(&mut self, group: usize) {
        self.group = group;
        // 分类切换后不能沿用上一组的滚动范围，下一帧再按实际宽度计算。
        self.scroll = 0;
        self.total = 0;
    }

    fn max_scroll(&self) -> usize {
        self.total.saturating_sub(self.viewport)
    }

    fn rows(&self, width: u16) -> Vec<Line<'static>> {
        let group = &GROUPS[self.group];
        let mut lines = Vec::new();
        let key_width = group
            .keys
            .iter()
            .map(|(key, _)| theme::display_width(key))
            .max()
            .unwrap_or(0)
            .clamp(14, 24);
        for (key, desc) in group.keys {
            if width >= 58 {
                // 键位列固定，续行只进入说明列；按显示列宽处理中文和组合键。
                let desc_width = width.saturating_sub(key_width as u16 + 3);
                for (index, row) in super::notice::wrap_text(desc, desc_width)
                    .into_iter()
                    .enumerate()
                {
                    let label = if index == 0 { *key } else { "" };
                    lines.push(Line::from(vec![
                        Span::styled(
                            format!(
                                "{label}{}",
                                " ".repeat(key_width - theme::display_width(label))
                            ),
                            theme::Theme::props(),
                        ),
                        Span::raw("   "),
                        Span::styled(row, theme::Theme::text()),
                    ]));
                }
            } else {
                for row in super::notice::wrap_text(key, width) {
                    lines.push(Line::from(Span::styled(row, theme::Theme::props())));
                }
                for row in super::notice::wrap_text(desc, width.saturating_sub(2).max(1)) {
                    lines.push(Line::from(format!("  {row}")));
                }
                lines.push(Line::from(""));
            }
        }
        lines
    }

    pub fn render(&mut self, f: &mut Frame, area: Rect) {
        theme::clear(f, area);
        let width = area.width.saturating_sub(2);
        let keys = if width >= 90 {
            " Tab / ←→ 分类 · 1–9 直达 · ↑↓ 滚动 · Esc/q 关闭 "
        } else if width >= 56 {
            " Tab 分类 · ↑↓ 滚动 · Esc/q 关闭 "
        } else if width >= 28 {
            " Tab 分类 · Esc 关闭 "
        } else {
            " Esc 关闭 "
        };
        let block = theme::block()
            .borders(Borders::ALL)
            .title(format!(
                " 快捷键 · {}/{} {} ",
                self.group + 1,
                GROUPS.len(),
                GROUPS[self.group].title
            ))
            .title_bottom(Line::from(theme::fit(keys, width as usize)).alignment(Alignment::Right))
            .border_style(theme::Theme::border_active());
        let inner = block.inner(area);
        f.render_widget(block, area);
        if inner.width < 3 || inner.height < 3 {
            self.viewport = 0;
            return;
        }
        let sections = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(2), Constraint::Min(1)])
            .split(inner);
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(if inner.width >= 90 {
                vec![Constraint::Length(18), Constraint::Min(1)]
            } else {
                vec![Constraint::Length(0), Constraint::Min(1)]
            })
            .split(sections[1]);
        if columns[0].width > 0 {
            let items: Vec<ListItem> = GROUPS
                .iter()
                .enumerate()
                .map(|(index, group)| ListItem::new(format!("{}  {}", index + 1, group.title)))
                .collect();
            f.render_stateful_widget(
                List::new(items)
                    .highlight_symbol("› ")
                    .highlight_style(theme::Theme::selected())
                    .block(theme::block().borders(Borders::RIGHT)),
                columns[0],
                &mut ListState::default().with_selected(Some(self.group)),
            );
        }
        let body = Rect {
            x: columns[1].x + 1,
            width: columns[1].width.saturating_sub(2),
            ..columns[1]
        };
        let lines = self.rows(body.width);
        self.total = lines.len();
        self.viewport = body.height as usize;
        self.scroll = self.scroll.min(self.max_scroll());
        let end = self.scroll.saturating_add(self.viewport).min(self.total);
        let status = format!(
            " {} · {} 个快捷键 · {}–{end}/{} 行",
            GROUPS[self.group].title,
            GROUPS[self.group].keys.len(),
            self.scroll + 1,
            self.total
        );
        f.render_widget(
            Paragraph::new(theme::fit(&status, sections[0].width as usize))
                .style(theme::Theme::dim()),
            sections[0],
        );
        f.render_widget(
            Paragraph::new(
                lines
                    .into_iter()
                    .skip(self.scroll)
                    .take(self.viewport)
                    .collect::<Vec<_>>(),
            ),
            body,
        );
        if self.total > self.viewport {
            let mut state = ScrollbarState::new(self.total)
                .position(self.scroll)
                .viewport_content_length(self.viewport);
            f.render_stateful_widget(
                theme::scrollbar(ScrollbarOrientation::VerticalRight),
                columns[1],
                &mut state,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn draw(panel: &mut HelpPanel, width: u16, height: u16) -> String {
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
        let frame = terminal.draw(|f| panel.render(f, f.area())).unwrap();
        frame
            .buffer
            .content
            .iter()
            .flat_map(|cell| cell.symbol().chars())
            .filter(|ch| !ch.is_whitespace() && !"│┃─╭╮╰╯".contains(*ch))
            .collect()
    }

    #[test]
    fn every_category_is_reachable_and_resets_scroll() {
        let mut panel = HelpPanel::default();
        for index in 0..GROUPS.len() {
            panel.handle_key(key(KeyCode::Char(char::from(b'1' + index as u8))));
            let text = draw(&mut panel, 100, 24);
            assert_eq!(panel.group, index);
            assert!(text.contains(GROUPS[index].title));
            let first_key: String = GROUPS[index].keys[0]
                .0
                .chars()
                .filter(|ch| !ch.is_whitespace())
                .collect();
            assert!(text.contains(&first_key));
            panel.handle_key(key(KeyCode::End));
        }
        panel.handle_key(key(KeyCode::Tab));
        assert_eq!(panel.group, 0);
        assert_eq!(panel.scroll, 0);
        panel.handle_key(key(KeyCode::BackTab));
        assert_eq!(panel.group, GROUPS.len() - 1);
        panel.handle_key(key(KeyCode::Right));
        assert_eq!(panel.group, 0);
    }

    #[test]
    fn all_descriptions_fit_and_can_be_read_after_resize() {
        let mut panel = HelpPanel::default();
        for (width, height) in [(30, 10), (64, 20), (100, 24), (140, 40)] {
            for index in 0..GROUPS.len() {
                panel.select_group(index);
                draw(&mut panel, width, height);
                let body_width = width - 4 - if width - 2 >= 90 { 18 } else { 0 };
                assert!(panel
                    .rows(body_width)
                    .iter()
                    .all(|line| line.width() <= body_width as usize));
                panel.handle_key(key(KeyCode::End));
                let text = draw(&mut panel, width, height);
                assert_eq!(panel.scroll, panel.max_scroll());
                let last = GROUPS[index].keys.last().unwrap().1;
                let suffix: String = last
                    .chars()
                    .rev()
                    .take(4)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect();
                let suffix: String = suffix.chars().filter(|ch| !ch.is_whitespace()).collect();
                assert!(
                    text.contains(&suffix),
                    "{width}×{height} / {} must show {suffix}: {text}",
                    GROUPS[index].title
                );
                assert!(text.contains("Esc"));
                panel.handle_key(key(KeyCode::Home));
                assert_eq!(panel.scroll, 0);
                if panel.total > panel.viewport {
                    panel.handle_key(key(KeyCode::PageDown));
                    assert!(panel.scroll > 0);
                    panel.handle_key(key(KeyCode::PageUp));
                    assert_eq!(panel.scroll, 0);
                }
            }
        }
        // 保留当前分类，从窄窗口底部放大后，滚动必须收敛到新的范围。
        panel.select_group(1);
        draw(&mut panel, 30, 10);
        panel.handle_key(key(KeyCode::End));
        draw(&mut panel, 140, 40);
        assert_eq!(panel.scroll, panel.max_scroll());
    }

    #[test]
    fn help_only_closes_on_explicit_close_keys() {
        let mut panel = HelpPanel::default();
        for code in [
            KeyCode::Enter,
            KeyCode::Char('C'),
            KeyCode::F(4),
            KeyCode::Down,
            KeyCode::Tab,
        ] {
            assert!(!panel.handle_key(key(code)));
        }
        for code in [
            KeyCode::Esc,
            KeyCode::Char('q'),
            KeyCode::Char('?'),
            KeyCode::F(1),
        ] {
            assert!(panel.handle_key(key(code)));
        }
    }

    fn desc_of(title: &str, key: &str) -> &'static str {
        GROUPS
            .iter()
            .find(|group| group.title == title)
            .unwrap()
            .keys
            .iter()
            .find(|(candidate, _)| *candidate == key)
            .unwrap()
            .1
    }

    #[test]
    fn help_preserves_exit_draft_and_destructive_key_contracts() {
        assert!(desc_of("查看", "Esc").contains("停留主面板"));
        assert!(desc_of("提交", "Esc").contains("保留草稿"));
        assert!(desc_of("其他", "q").contains("退出主面板"));
        assert!(desc_of("改动", "d").contains("高危"));
        assert!(desc_of("改动", "D").contains("中危"));
        for group in GROUPS {
            for (_, desc) in group.keys {
                assert!(!desc.contains("R 回退"));
            }
        }
    }
}
