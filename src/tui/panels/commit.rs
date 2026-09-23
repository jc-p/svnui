//! 提交面板 —— **只负责提交信息**。
//!
//! ## 为什么不在这里选文件
//!
//! 早期版本把文件勾选也放进弹框，结果很难用：
//! - 弹框挡住了 diff，你想"看一眼改动再决定勾不勾"就得反复进出
//! - 弹框里只能用 ↑↓ 一个个挪，看不到文件在目录里的位置
//! - 全屏弹框里选文件，和外面那棵树是两套心智模型
//!
//! 改成**在文件树上直接 Space 勾选**（边看 diff 边挑），
//! 弹框只干一件事：写提交信息。这也符合 lazygit 的习惯。
//!
//! ```text
//! ┌─────────── 提交（3 个文件）────────────┐
//! │  src/main.rs                          │
//! │  src/lib.rs                           │
//! │  docs/guide.md                        │
//! │ ───────────────────────────────────── │
//! │ 提交信息                               │
//! │ ┌───────────────────────────────────┐ │
//! │ │ 修复状态同步                        │ │
//! │ └───────────────────────────────────┘ │
//! │ Ctrl+S 提交   Esc 退出                  │
//! └───────────────────────────────────────┘
//! ```

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
    Frame,
};
use tui_textarea::TextArea;

use super::commit_gen::{self, Change};

use crate::tui::theme;

/// 提交面板。
///
/// 文件列表是**只读**的（勾选在文件树上做），这里只展示"将要提交什么"，
/// 让用户最后确认一眼范围。
pub struct CommitPanel {
    /// 将要提交的文件（相对路径，用于展示）。
    files: Vec<String>,
    /// 多行提交信息编辑器。
    textarea: TextArea<'static>,
    /// 底部提示（错误信息等）。
    hint: Option<String>,
    /// 文件列表滚动（文件多时）。
    scroll: usize,
    /// 本次改动（相对路径 + 状态符号），`Ctrl+G` 生成提交信息的输入。
    changes: Vec<Change>,
    /// 后台生成的候选（`Ctrl+G`）。空则用本地规则现算。
    cands: Vec<String>,
    /// 上次生成的候选（下标 + 文本）。`Some` 表示框里现在是生成出来的内容。
    gen: Option<(usize, String)>,
    /// 底部提示（绿色，非错误）。生成反馈走这里，红色留给真错误。
    notice: Option<String>,
}

impl CommitPanel {
    /// `files` 是要提交的文件显示名（已美化过的路径）。
    /// `draft` 是上次退出时存下的内容，有的话直接填进去。
    pub fn new(files: Vec<String>, draft: Option<String>, changes: Vec<Change>) -> Self {
        let mut textarea = TextArea::default();
        textarea.set_block(
            Block::default()
                .borders(Borders::ALL)
                .title(" 提交信息 "),
        );
        textarea.set_placeholder_text("写点什么说明这次改动（Ctrl+G 生成、Ctrl+S 提交）");

        // 恢复草稿：按 Enter 前按过 Esc 的话，内容不该丢。
        // 只有真的有非空白内容才填 —— 空串会让 placeholder 消失，
        // 看起来像"有个空行"，反而奇怪。
        if let Some(d) = draft {
            if !d.trim().is_empty() {
                for (i, line) in d.lines().enumerate() {
                    if i > 0 {
                        textarea.insert_newline();
                    }
                    textarea.insert_str(line);
                }
            }
        }

        Self {
            files,
            textarea,
            hint: None,
            changes,
            cands: Vec::new(),
            gen: None,
            notice: None,
            scroll: 0,
        }
    }

    pub fn set_hint(&mut self, hint: impl Into<String>) {
        self.hint = Some(hint.into());
    }

    /// 提交信息（去首尾空白后为空则视为未填）。
    pub fn message(&self) -> String {
        self.textarea.lines().join("\n").trim().to_string()
    }

    pub fn textarea_mut(&mut self) -> &mut TextArea<'static> {
        &mut self.textarea
    }

    /// 在光标处插入文本（粘贴用）。
    pub fn insert_str(&mut self, s: &str) {
        // 换行统一：Windows 剪贴板用 CRLF，老 Mac 用单个 CR。
        // 不处理的话每行末尾会留一个 ^M，提交信息里带着很脏。
        // （注释里不写转义字面量 —— 会被当成真实回车把注释截断。）
        let cleaned = s.replace("\r\n", "\n").replace('\r', "\n");

        // 超长截断：误粘一个几 MB 的文件可以把 TextArea 拖到卡死
        // （每次渲染都要重排所有行）。
        // 必须按**字符边界**切 —— 直接切字节下标会在中文中间断开，panic。
        const MAX_CHARS: usize = 64 * 1024;
        let cleaned = if cleaned.chars().count() > MAX_CHARS {
            let end = cleaned
                .char_indices()
                .nth(MAX_CHARS)
                .map(|(i, _)| i)
                .unwrap_or(cleaned.len());
            cleaned[..end].to_string()
        } else {
            cleaned
        };

        self.textarea.insert_str(&cleaned);
    }

    /// 清掉底部提示。
    ///
    /// 粘贴（或任何编辑）之后，之前那条"提交信息不能为空"就过期了，
    /// 继续挂着会让用户以为刚粘进去的内容没生效。
    pub fn clear_hint(&mut self) {
        self.hint = None;
        // 只清提示，不清 gen：移动光标也算"编辑"，但内容没变，
        // 这时再按 Ctrl+G 应该继续轮换，而不是叫人先清空。
        // 真改动了内容也没关系 —— next_suggestion 里比对文本即可识别。
        self.notice = None;
    }

    pub fn set_notice(&mut self, notice: impl Into<String>) {
        self.notice = Some(notice.into());
    }

    /// 是否已有外部候选（后台生成的结果）。
    pub fn has_candidates(&self) -> bool {
        !self.cands.is_empty()
    }

    /// 填入后台生成的候选。
    ///
    /// 约定：模型候选在前、规则候选在后。这样模型失败时继续按
    /// `Ctrl+G` 还能轮换到规则版本。
    pub fn set_candidates(&mut self, cands: Vec<String>) {
        self.cands = cands;
    }

    /// 生成 / 轮换提交信息（`Ctrl+G`）。返回给底部显示的一句反馈。
    ///
    /// ## 只在两种情况下动文本
    ///
    /// - 提交框是空的
    /// - 当前内容**就是**上次生成的那条（用户没手动改过）
    ///
    /// 手填过的内容一律不覆盖 —— 生成的是"起点"，
    /// 把别人写了一半的东西顶掉是最招人烦的行为。
    /// 想重新生成自己清空，比偷偷覆盖好判断。
    pub fn next_suggestion(&mut self) -> String {
        if self.changes.is_empty() {
            return "没有可分析的改动".to_string();
        }
        let cands = if self.cands.is_empty() {
            commit_gen::suggest(&self.changes)
        } else {
            self.cands.clone()
        };
        let cur = self.textarea.lines().join("\n").trim().to_string();
        let idx = match &self.gen {
            Some((i, last)) if cur == *last => (*i + 1) % cands.len(),
            Some(_) => {
                return "内容已改过，清空后再按 Ctrl+G 可重新生成".to_string();
            }
            None if cur.is_empty() => 0,
            None => {
                return "已有内容，清空后再按 Ctrl+G 可重新生成".to_string();
            }
        };
        let text = cands[idx].clone();
        self.textarea = Self::build_editor(&text);
        self.gen = Some((idx, text));
        self.hint = None;
        format!("已生成 {}/{}，再按 Ctrl+G 换下一个", idx + 1, cands.len())
    }

    /// 按给定文本重建编辑器。
    ///
    /// 为什么整体重建而不是就地改：TextArea 没有公开的"全选替换"，
    /// 靠模拟按键去删既绕又容易删不干净。这里用的都是 `new()` 里
    /// 已经在用的 API，行为可预期。
    fn build_editor(text: &str) -> TextArea<'static> {
        let mut ta = TextArea::default();
        ta.set_block(
            Block::default()
                .borders(Borders::ALL)
                .title(" 提交信息 "),
        );
        ta.set_placeholder_text("写点什么说明这次改动（Ctrl+G 生成、Ctrl+S 提交）");
        for (i, line) in text.lines().enumerate() {
            if i > 0 {
                ta.insert_newline();
            }
            ta.insert_str(line);
        }
        ta
    }

    pub fn scroll_down(&mut self) {
        self.scroll = self.scroll.saturating_add(1);
    }

    pub fn scroll_up(&mut self) {
        self.scroll = self.scroll.saturating_sub(1);
    }

    /// 居中弹框。
    fn centered(area: Rect) -> Rect {
        let w = (area.width * 70 / 100).max(40).min(area.width);
        let h = (area.height * 70 / 100).max(10).min(area.height);
        Rect {
            x: area.x + (area.width.saturating_sub(w)) / 2,
            y: area.y + (area.height.saturating_sub(h)) / 2,
            width: w,
            height: h,
        }
    }

    pub fn render(&mut self, f: &mut Frame, area: Rect) {
        let pop = Self::centered(area);
        f.render_widget(Clear, pop);

        let n = self.files.len();
        let block = Block::default()
            .borders(Borders::ALL)
            .title(Line::from(Span::styled(
                format!(" 提交（{} 个文件） ", n),
                theme::Theme::title(),
            )))
            .border_style(theme::Theme::border_active());

        let inner = block.inner(pop);
        f.render_widget(block, pop);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(4), // 文件清单
                Constraint::Min(4),    // 提交信息
                Constraint::Length(1), // 提示/按键
            ])
            .split(inner);

        // ── 文件清单（只读）──
        // 只显示前几行 + "还有 N 个"，完整范围在文件树上看。
        // 列一长串会挤掉提交信息的空间，而那才是这里的主角。
        let shown: Vec<ListItem> = self
            .files
            .iter()
            .take(chunks[0].height as usize)
            .map(|s| {
                ListItem::new(Line::from(Span::styled(
                    format!("  {}", s),
                    Style::default().fg(ratatui::style::Color::Gray),
                )))
            })
            .collect();
        let rest = self.files.len().saturating_sub(chunks[0].height as usize);
        let mut list = List::new(shown);
        if rest > 0 {
            // 提示还有更多，避免用户以为只提交这几条
            list = list.block(
                Block::default()
                    .borders(Borders::TOP)
                    .title(Span::styled(
                        format!(" 还有 {} 个… ", rest),
                        theme::Theme::dim(),
                    )),
            );
        }
        f.render_widget(list, chunks[0]);

        // ── 提交信息 ──
        f.render_widget(&self.textarea, chunks[1]);

        // ── 底部：提示优先，否则按键说明 ──
        let line = match &self.hint {
            Some(h) => Line::from(Span::styled(
                format!(" {}", h),
                Style::default().fg(ratatui::style::Color::LightRed),
            )),
            None => match &self.notice {
                Some(n) => Line::from(Span::styled(
                    format!(" {}", n),
                    Style::default().fg(ratatui::style::Color::LightGreen),
                )),
                None => Line::from(vec![
                    Span::styled(" Ctrl+S ", theme::Theme::title()),
                    Span::styled("提交    ", theme::Theme::dim()),
                    Span::styled(" Ctrl+G ", theme::Theme::title()),
                    Span::styled("生成    ", theme::Theme::dim()),
                    Span::styled(" Esc ", theme::Theme::title()),
                    Span::styled("退出", theme::Theme::dim()),
                ]),
            },
        };
        f.render_widget(Paragraph::new(line), chunks[2]);
    }
}
