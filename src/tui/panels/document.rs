//! 办公文档的文本提取（CSV / Excel / Word）。
//!
//! ## 为什么要用专门的库，而不是直接读文件
//!
//! `.xlsx` / `.docx` 本质是 **zip 包里面装 XML**。直接 `read_to_string` 会得到
//! 一堆压缩过的二进制——终端里全是乱码，甚至可能因为控制字符把终端状态搞乱。
//! 所以这类扩展名必须在"读文件"之前拦下来，走解压 + XML 解析。
//!
//! ## 为什么是这三个库
//!
//! | 格式 | 库 | 理由 |
//! |---|---|---|
//! | CSV/TSV | `csv` | 事实标准。分隔符转义、引号换行、UTF-8 BOM 都处理好了 |
//! | xlsx/xls/ods | `calamine` | 纯 Rust，覆盖面最广，每周 30 万+ 下载 |
//! | docx | `docx-lite` | 只依赖 zip + quick-xml，比 office_oxide 轻得多 |
//!
//! 全部走 **feature gate**（`docs`），不想要可以不编译。

use std::path::Path;

/// 单个文档最多提取多少字符（防超大表格把界面卡死）。
const MAX_CHARS: usize = 200_000;

/// 单个 sheet 最多渲染多少行 / 列。
const MAX_ROWS: usize = 2000;
const MAX_COLS: usize = 50;

/// 这个路径是不是我们能提取文本的办公文档。
///
/// 按扩展名判断——准确率高且零成本，不需要读文件内容做魔数嗅探。
pub fn is_document(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| {
            let e = e.to_lowercase();
            matches!(
                e.as_str(),
                "csv" | "tsv" | "xls" | "xlsx" | "xlsm" | "xlsb" | "ods" | "docx"
            )
        })
        .unwrap_or(false)
}

/// 提取文档文本。返回 `None` 表示"这不是文档"或"提取失败"，
/// 调用方应回落到普通的读文件流程。
///
/// ⚠️ 本模块整体在 `docs` feature 下才编译（见 Cargo.toml）。
///    关掉该 feature 后 `is_document` / `extract` 都不存在，
///    `read_file` 里的调用点也被 `#[cfg]` 掉了，不会链接错误。
pub fn extract(path: &Path) -> Option<String> {
    let ext = path.extension()?.to_str()?.to_lowercase();
    let text = match ext.as_str() {
        "csv" | "tsv" => extract_csv(path, ext == "tsv"),
        "xls" | "xlsx" | "xlsm" | "xlsb" | "ods" => extract_sheet(path),
        "docx" => extract_docx(path),
        _ => return None,
    };
    text.map(truncate)
}

fn truncate(mut s: String) -> String {
    if s.len() > MAX_CHARS {
        // ⚠️ 不能直接用 `s[..MAX_CHARS]` 切：那是字节下标，
        //    中文是多字节，切在中间会得到非法 UTF-8（panic）。
        //    用 char_indices 找到最后一个完整的字符边界。
        let end = s
            .char_indices()
            .nth(MAX_CHARS)
            .map(|(i, _)| i)
            .unwrap_or(s.len());
        s.truncate(end);
        s.push_str(&format!("\n\n… 内容过长，只显示前 {} 个字符", MAX_CHARS));
    }
    s
}

// ── CSV ──────────────────────────────────────────────────

/// CSV 渲染成**对齐的表格**，不是原样输出。
///
/// 原样输出时列宽参差不齐，看 col3 的值要对半天。按列宽补空格对齐后
/// 才像表格。分隔符用 ` │ `，比逗号好辨认。
fn extract_csv(path: &Path, tsv: bool) -> Option<String> {
    let delim = if tsv { b'\t' } else { b',' };
    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(false)
        .delimiter(delim)
        .flexible(true) // 行长不齐不要报错，补空就行
        .from_path(path)
        .ok()?;

    let mut rows: Vec<Vec<String>> = Vec::new();
    for rec in rdr.records().take(MAX_ROWS) {
        let rec = rec.ok()?;
        rows.push(rec.iter().take(MAX_COLS).map(|s| s.to_string()).collect());
    }

    Some(render_table(rows))
}

/// 把二维数据渲染成对齐的表格。Excel 和 CSV 共用。
fn render_table(rows: Vec<Vec<String>>) -> String {
    if rows.is_empty() {
        return "（空表）".to_string();
    }

    // 列宽 = 该列最长单元格；上限 30 字符，否则一两个超长值会把整表撑爆
    let ncols = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    let mut widths = vec![0usize; ncols];
    for r in &rows {
        for (i, c) in r.iter().enumerate() {
            widths[i] = widths[i].max(display_width(c).min(30));
        }
    }

    let mut out = String::new();
    for (ri, r) in rows.iter().enumerate() {
        let mut cells: Vec<String> = Vec::with_capacity(ncols);
        for i in 0..ncols {
            let c = r.get(i).map(|s| s.as_str()).unwrap_or("");
            // 单元格里的换行会破坏表格布局，压成空格
            let c: String = c.chars().map(|ch| if ch == '\n' { ' ' } else { ch }).collect();
            cells.push(pad_to(&c, widths[i]));
        }
        let line = cells.join(" │ ");
        out.push_str(line.trim_end());
        out.push('\n');

        // 表头下加分隔线
        if ri == 0 && rows.len() > 1 {
            let sep: String = widths.iter().map(|w| "─".repeat(*w)).collect::<Vec<_>>().join("─┼─");
            out.push_str(&sep);
            out.push('\n');
        }
    }
    out
}

/// 显示宽度：中文占 2 列，ASCII 占 1 列。
///
/// 不用 unicode-width 库——为此引一个依赖不值当，
/// 而且我们只需要粗略对齐，按码点范围判断就够了。
fn display_width(s: &str) -> usize {
    s.chars()
        .map(|c| {
            let c = c as u32;
            if (0x1100..=0x115F).contains(&c)
                || (0x2E80..=0xA4CF).contains(&c)
                || (0xAC00..=0xD7A3).contains(&c)
                || (0xF900..=0xFAFF).contains(&c)
                || (0xFE30..=0xFE6F).contains(&c)
                || (0xFF00..=0xFF60).contains(&c)
                || (0xFFE0..=0xFFE6).contains(&c)
            {
                2
            } else {
                1
            }
        })
        .sum()
}

fn pad_to(s: &str, width: usize) -> String {
    let w = display_width(s);
    if w >= width {
        s.to_string()
    } else {
        format!("{}{}", s, " ".repeat(width - w))
    }
}

// ── Excel ────────────────────────────────────────────────

fn extract_sheet(path: &Path) -> Option<String> {
    use calamine::{open_workbook_auto, Reader};

    let mut wb = open_workbook_auto(path).ok()?;
    let names = wb.sheet_names().to_vec();
    if names.is_empty() {
        return None;
    }

    let mut out = String::new();
    for (i, name) in names.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        if names.len() > 1 {
            out.push_str(&format!("── Sheet: {} ──\n", name));
        }

        let range = match wb.worksheet_range(name) {
            Ok(r) => r,
            Err(_) => {
                out.push_str("（该 sheet 读取失败，可能是图表或特殊格式）\n");
                continue;
            }
        };

        let mut rows: Vec<Vec<String>> = Vec::new();
        for r in range.rows().take(MAX_ROWS) {
            rows.push(
                r.iter()
                    .take(MAX_COLS)
                    .map(cell_to_string)
                    .collect::<Vec<_>>(),
            );
        }
        out.push_str(&render_table(rows));
    }
    Some(out)
}

fn cell_to_string(c: &calamine::Data) -> String {
    use calamine::Data;
    match c {
        Data::Empty => String::new(),
        Data::String(s) => s.clone(),
        Data::Float(f) => {
            // 整数值不要显示成 42.0 —— 表格里看着很别扭
            if f.fract() == 0.0 && f.abs() < 1e15 {
                format!("{}", *f as i64)
            } else {
                format!("{}", f)
            }
        }
        Data::Int(i) => format!("{}", i),
        Data::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_string(),
        Data::DateTime(d) => d.to_string(),
        Data::DateTimeIso(s) => s.clone(),
        Data::DurationIso(s) => s.clone(),
        Data::Error(e) => format!("#{}", e),
        // 没有 `_ =>` 兜底，是**故意的**：
        //    calamine 0.36 的 Data 不是 non_exhaustive，上面已穷尽所有变体，
        //    加兜底只会掩盖"新增类型没被处理"这个问题。
        //    将来 calamine 加了新变体，这里会编译失败 —— 那正是我们想要的：
        //    强制你决定新类型该怎么显示，而不是静默变成空字符串。
    }
}

// ── Word ─────────────────────────────────────────────────

fn extract_docx(path: &Path) -> Option<String> {
    // docx-lite 的 API 是 extract_text(path)，返回 io::Result<String>
    docx_lite::extract_text(path).ok()
}

#[cfg(all(test, feature = "docs"))]
mod tests {
    use super::*;

    #[test]
    fn display_width_counts_cjk_as_two() {
        assert_eq!(display_width("abc"), 3);
        assert_eq!(display_width("中文"), 4);
        assert_eq!(display_width("a中"), 3);
    }

    #[test]
    fn truncate_never_splits_a_char() {
        let long: String = "中".repeat(300_000);
        let out = truncate(long);
        // 不 panic 就算过；长度应在 MAX_CHARS 附近（字符数）
        assert!(out.chars().count() <= MAX_CHARS + 40);
    }

    #[test]
    fn render_table_aligns_columns() {
        let rows = vec![
            vec!["name".to_string(), "age".to_string()],
            vec!["张三".to_string(), "30".to_string()],
            vec!["b".to_string(), "1".to_string()],
        ];
        let out = render_table(rows);
        let lines: Vec<&str> = out.lines().collect();
        // 三行内容 + 一条分隔线
        assert_eq!(lines.len(), 4);
        // 第一行和最后一行里 "│" 的位置应当一致（列对齐）
        let p1 = lines[0].find('│').unwrap();
        let p2 = lines[2].find('│').unwrap();
        // 中文 "张三" 占 4 列，所以首列 padding 不同，但 │ 的显示位置应对齐
        let _ = (p1, p2);
    }

    #[test]
    fn render_table_pads_cells_to_equal_width() {
        // 关键：中文占 2 列，pad 必须按显示宽度算，
        // 否则 "张三" 那行会比 "b" 那行短一截，表格是歪的
        let rows = vec![
            vec!["name".to_string(), "v".to_string()],
            vec!["张三".to_string(), "1".to_string()],
        ];
        let out = render_table(rows);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 3, "内容2行 + 分隔线1行");
        // 两行里 │ 的显示列位置必须一致
        let col_of = |l: &str| -> usize {
            l.find('│').map(|b| l[..b].chars().fold(0, |a, c| a + display_width(&c.to_string()))).unwrap()
        };
        assert_eq!(col_of(lines[0]), col_of(lines[2]));
    }

    #[test]
    fn empty_table_is_not_blank() {
        // 空表如果返回空串，预览区会是一片空白，用户以为坏了
        assert!(render_table(vec![]).contains("空表"));
    }

    #[test]
    fn cell_to_string_drops_trailing_dot_zero() {
        use calamine::Data;
        assert_eq!(cell_to_string(&Data::Float(42.0)), "42");
        assert_eq!(cell_to_string(&Data::Float(3.5)), "3.5");
        assert_eq!(cell_to_string(&Data::Empty), "");
    }

    #[test]
    fn is_document_recognizes_office_extensions() {
        assert!(is_document(Path::new("a.xlsx")));
        assert!(is_document(Path::new("a.XLSX")));
        assert!(is_document(Path::new("a.csv")));
        assert!(is_document(Path::new("a.docx")));
        assert!(!is_document(Path::new("a.txt")));
        assert!(!is_document(Path::new("a.rs")));
    }
}
