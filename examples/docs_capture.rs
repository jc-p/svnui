//! 用临时 SVN 工作副本生成 README 配图；不读取真实项目，不调用模型或提交远端。
//! 输出 SVG 后用 rsvg-convert 转成 PNG，复现步骤见 docs/images/README.md。

#[cfg(feature = "tui")]
mod capture {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::{
        backend::TestBackend,
        style::{Color, Modifier},
        Terminal,
    };
    use std::{
        fmt::Write,
        path::Path,
        process::Command,
        time::{Duration, Instant},
    };
    use svnui::{svn::Svn, tui::app::App};

    type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

    fn svn(args: &[&str], cwd: &Path) -> Result<()> {
        let output = Command::new("svn").args(args).current_dir(cwd).output()?;
        if !output.status.success() {
            return Err(format!("svn: {}", String::from_utf8_lossy(&output.stderr)).into());
        }
        Ok(())
    }

    fn key(app: &mut App, code: KeyCode) -> Result<()> {
        app.handle_key(KeyEvent::new(code, KeyModifiers::NONE))?;
        Ok(())
    }

    fn settle(app: &mut App) {
        // 包含只读预览工作线程；仅 is_busy 变为空闲不代表预览已返回。
        let deadline = Instant::now() + Duration::from_millis(800);
        while Instant::now() < deadline {
            app.tick();
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn escape(text: &str) -> String {
        text.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
    }

    fn color(color: Color, default: &str) -> String {
        let palette = [
            "#1e1e2e", "#f38ba8", "#a6e3a1", "#f9e2af", "#89b4fa", "#cba6f7", "#94e2d5", "#bac2de",
            "#585b70", "#f38ba8", "#a6e3a1", "#f9e2af", "#89b4fa", "#cba6f7", "#94e2d5", "#cdd6f4",
        ];
        match color {
            Color::Reset => default.into(),
            Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
            Color::Indexed(index) if index < 16 => palette[index as usize].into(),
            Color::Indexed(index) if index >= 232 => {
                let v = 8 + (index - 232) * 10;
                format!("#{v:02x}{v:02x}{v:02x}")
            }
            Color::Indexed(index) => {
                let n = index - 16;
                let levels = [0, 95, 135, 175, 215, 255];
                format!(
                    "#{:02x}{:02x}{:02x}",
                    levels[(n / 36) as usize],
                    levels[((n / 6) % 6) as usize],
                    levels[(n % 6) as usize]
                )
            }
            other => {
                let index = match other {
                    Color::Black => 0,
                    Color::Red => 1,
                    Color::Green => 2,
                    Color::Yellow => 3,
                    Color::Blue => 4,
                    Color::Magenta => 5,
                    Color::Cyan => 6,
                    Color::Gray => 7,
                    Color::DarkGray => 8,
                    Color::LightRed => 9,
                    Color::LightGreen => 10,
                    Color::LightYellow => 11,
                    Color::LightBlue => 12,
                    Color::LightMagenta => 13,
                    Color::LightCyan => 14,
                    Color::White => 15,
                    _ => 7,
                };
                palette[index].into()
            }
        }
    }

    fn snapshot(app: &mut App, output: &Path, expected: &str) -> Result<()> {
        const COLS: u16 = 128;
        const ROWS: u16 = 34;
        let mut terminal = Terminal::new(TestBackend::new(COLS, ROWS))?;
        let frame = terminal.draw(|f| app.render(f, f.area()))?;
        let text: String = frame
            .buffer
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        if !text.contains(expected) {
            std::fs::write(output.with_extension("txt"), &text)?;
            return Err(format!("配图缺少预期界面内容：{expected}").into());
        }
        let mut svg = format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\">", COLS*10+32, ROWS*22+32, COLS*10+32, ROWS*22+32);
        svg.push_str("<rect width=\"100%\" height=\"100%\" rx=\"12\" fill=\"#1e1e2e\"/><g font-family=\"Maple Mono NF CN,monospace\" font-size=\"17\">");
        for row in 0..ROWS {
            // 背景按连续颜色合并；字形按真实终端列定位，保留中英文宽度。
            let mut x = 0;
            while x < COLS {
                let cell = &frame.buffer[(x, row)];
                let bg = if cell.modifier.contains(Modifier::REVERSED) {
                    cell.fg
                } else {
                    cell.bg
                };
                let start = x;
                x += 1;
                while x < COLS {
                    let next = &frame.buffer[(x, row)];
                    let next_bg = if next.modifier.contains(Modifier::REVERSED) {
                        next.fg
                    } else {
                        next.bg
                    };
                    if next_bg != bg {
                        break;
                    }
                    x += 1;
                }
                writeln!(
                    svg,
                    "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"22\" fill=\"{}\"/>",
                    16 + start * 10,
                    16 + row * 22,
                    (x - start) * 10,
                    color(bg, "#1e1e2e")
                )?;
            }
            for x in 0..COLS {
                let cell = &frame.buffer[(x, row)];
                if cell.symbol().trim().is_empty() {
                    continue;
                }
                let fg = if cell.modifier.contains(Modifier::REVERSED) {
                    cell.bg
                } else {
                    cell.fg
                };
                let weight = if cell.modifier.contains(Modifier::BOLD) {
                    "bold"
                } else {
                    "normal"
                };
                writeln!(
                    svg,
                    "<text x=\"{}\" y=\"{}\" fill=\"{}\" font-weight=\"{weight}\">{}</text>",
                    16 + x * 10,
                    33 + row * 22,
                    color(fg, "#cdd6f4"),
                    escape(cell.symbol())
                )?;
            }
        }
        svg.push_str("</g></svg>\n");
        std::fs::write(output, svg)?;
        Ok(())
    }

    pub fn run() -> Result<()> {
        let output = std::env::args_os().nth(1).ok_or("需要 SVG 输出目录参数")?;
        let output = Path::new(&output);
        std::fs::create_dir_all(output)?;
        let temp = tempfile::Builder::new()
            .prefix("svnui-demo-")
            .tempdir_in(std::env::temp_dir())?;
        let admin = Command::new("svnadmin")
            .args(["create", "repo"])
            .current_dir(temp.path())
            .output()?;
        if !admin.status.success() {
            return Err("svnadmin create 失败".into());
        }
        let url = format!("file://{}/repo", temp.path().display());
        svn(
            &["checkout", "--non-interactive", &url, "demo"],
            temp.path(),
        )?;
        let wc = temp.path().join("demo");
        std::fs::create_dir_all(wc.join("src"))?;
        std::fs::write(
            wc.join("app.rs"),
            "fn main() {\n    println!(\"hello\");\n}\n",
        )?;
        std::fs::write(
            wc.join("src/settings.toml"),
            "theme = \"terminal\"\nauto_refresh = false\n",
        )?;
        std::fs::write(
            wc.join("README.md"),
            "# 示例项目\n\n使用 svnui 查看改动。\n",
        )?;
        svn(&["add", "app.rs", "src", "README.md"], &wc)?;
        svn(
            &[
                "commit",
                "--non-interactive",
                "--username",
                "demo",
                "-m",
                "feat: 初始化示例项目",
            ],
            &wc,
        )?;
        svn(&["update", "--non-interactive"], &wc)?;
        std::fs::write(
            wc.join("app.rs"),
            "fn main() {\n    println!(\"hello, svnui\");\n}\n",
        )?;
        std::fs::write(
            wc.join("src/settings.toml"),
            "theme = \"catppuccin\"\nauto_refresh = true\n",
        )?;
        std::fs::write(
            wc.join("README.md"),
            "# 示例项目\n\nSpace 选择文件，C 编写提交说明。\n",
        )?;
        std::fs::write(wc.join("notes.txt"), "待整理的本地笔记\n")?;
        // 仅为本示例进程设置配置与缓存目录，隔离用户模型、项目记录和状态缓存。
        std::env::set_var("SVNUI_CONFIG", temp.path().join("config.toml"));
        std::env::set_var("SVNR_CACHE_DIR", temp.path().join("cache"));
        // 文档统一展示彩色主题；只影响生成器进程，不改变终端或用户配置。
        std::env::remove_var("NO_COLOR");
        std::fs::write(
            temp.path().join("config.toml"),
            "[ui]\ntheme = \"catppuccin\"\n",
        )?;
        svnui::tui::theme::initialize("catppuccin");
        let mut app = App::new(Svn::discover(&wc, Duration::from_secs(5), false)?)?;
        settle(&mut app);
        // 只通过公开键盘入口移动；以预览内容确认文件，避免依赖树排序和异步时序。
        for _ in 0..12 {
            let mut terminal = Terminal::new(TestBackend::new(128, 34))?;
            let frame = terminal.draw(|f| app.render(f, f.area()))?;
            let text: String = frame
                .buffer
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect();
            if text.contains("hello, svnui") {
                break;
            }
            key(&mut app, KeyCode::Down)?;
            settle(&mut app);
        }
        snapshot(&mut app, &output.join("overview.svg"), "hello, svnui")?;
        key(&mut app, KeyCode::Char(' '))?;
        key(&mut app, KeyCode::Char('C'))?;
        app.handle_paste(
            "feat: 优化欢迎语与终端体验\n\n- 更新启动提示\n- 保留清晰的变更范围".into(),
        );
        snapshot(&mut app, &output.join("commit.svg"), "Ctrl+S")?;
        key(&mut app, KeyCode::Esc)?;
        key(&mut app, KeyCode::F(1))?;
        snapshot(&mut app, &output.join("help.svg"), "Shift+Tab")?;
        println!("已从本地示例工作副本生成 overview / commit / help SVG");
        Ok(())
    }
}

#[cfg(feature = "tui")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    capture::run()
}

#[cfg(not(feature = "tui"))]
fn main() {
    eprintln!("配图示例需要默认功能或 --features tui");
    std::process::exit(1);
}
