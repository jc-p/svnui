//! 用终端标识和系统字符尺寸选择图片协议，不另起 stdin 读取线程。

use image::{DynamicImage, Rgba};
use ratatui_image::{
    picker::ProtocolType,
    protocol::{
        halfblocks::Halfblocks, iterm2::Iterm2, kitty::StatefulKitty, ImageSource,
        StatefulProtocol, StatefulProtocolType,
    },
};
use std::sync::atomic::{AtomicU32, Ordering};

#[derive(Clone)]
pub struct ImageSupport {
    font_size: (u16, u16),
    protocol: ProtocolType,
}

impl ImageSupport {
    pub fn halfblocks() -> Self {
        Self {
            font_size: (10, 20),
            protocol: ProtocolType::Halfblocks,
        }
    }

    pub fn new_resize_protocol(&self, image: DynamicImage) -> StatefulProtocol {
        // 使用公开构造器，避开 Picker 的 stdin 探测及 tmux 设置副作用。
        let source = ImageSource::new(image, self.font_size, Rgba([0, 0, 0, 0]));
        let protocol = match self.protocol {
            ProtocolType::Kitty => {
                static NEXT_ID: AtomicU32 = AtomicU32::new(1);
                // 同一进程的图片使用独立 ID；进程前缀降低不同进程间的冲突机会。
                let id = loop {
                    let id = std::process::id().rotate_left(16)
                        ^ NEXT_ID.fetch_add(1, Ordering::Relaxed);
                    if id != 0 {
                        break id;
                    }
                };
                StatefulProtocolType::Kitty(StatefulKitty::new(id, false))
            }
            ProtocolType::Iterm2 => StatefulProtocolType::ITerm2(Iterm2::default()),
            _ => StatefulProtocolType::Halfblocks(Halfblocks::default()),
        };
        StatefulProtocol::new(source, self.font_size, protocol)
    }
}

pub fn detect() -> ImageSupport {
    let font_size = crossterm::terminal::window_size()
        .ok()
        .and_then(|size| {
            size.width
                .checked_div(size.columns)
                .zip(size.height.checked_div(size.rows))
        })
        .filter(|(width, height)| *width > 0 && *height > 0)
        .unwrap_or((10, 20));

    let program = std::env::var("TERM_PROGRAM").unwrap_or_default();
    let term = std::env::var("TERM").unwrap_or_default();
    let tmux = std::env::var_os("TMUX").is_some_and(|value| !value.is_empty());
    ImageSupport {
        font_size,
        protocol: protocol_for(&program, &term, tmux),
    }
}

fn protocol_for(program: &str, term: &str, tmux: bool) -> ProtocolType {
    if tmux || term.starts_with("screen") || term.starts_with("tmux") {
        return ProtocolType::Halfblocks;
    }
    match program.to_ascii_lowercase().as_str() {
        "ghostty" | "kitty" => ProtocolType::Kitty,
        "iterm.app" | "wezterm" | "rio" => ProtocolType::Iterm2,
        _ if matches!(term, "xterm-kitty" | "xterm-ghostty") => ProtocolType::Kitty,
        _ => ProtocolType::Halfblocks,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_protocols_encode_without_terminal_queries() {
        for protocol in [
            ProtocolType::Halfblocks,
            ProtocolType::Kitty,
            ProtocolType::Iterm2,
        ] {
            let support = ImageSupport {
                font_size: (10, 20),
                protocol,
            };
            let mut state = support.new_resize_protocol(DynamicImage::new_rgb8(32, 16));
            let mut terminal =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(20, 6)).unwrap();
            terminal
                .draw(|f| {
                    f.render_stateful_widget(
                        ratatui_image::StatefulImage::default(),
                        f.area(),
                        &mut state,
                    )
                })
                .unwrap();
            assert!(state.last_encoding_result().unwrap().is_ok());
        }
    }

    #[test]
    fn known_terminals_choose_their_image_protocol() {
        for program in ["ghostty", "kitty"] {
            assert_eq!(
                protocol_for(program, "xterm-256color", false),
                ProtocolType::Kitty
            );
        }
        for program in ["iTerm.app", "WezTerm", "rio"] {
            assert_eq!(
                protocol_for(program, "xterm-256color", false),
                ProtocolType::Iterm2
            );
        }
        assert_eq!(protocol_for("", "xterm-kitty", false), ProtocolType::Kitty);
    }

    #[test]
    fn unknown_terminals_and_multiplexers_use_compatible_blocks() {
        assert_eq!(
            protocol_for("", "xterm-256color", false),
            ProtocolType::Halfblocks
        );
        assert_eq!(
            protocol_for("Apple_Terminal", "xterm-256color", false),
            ProtocolType::Halfblocks
        );
        assert_eq!(
            protocol_for("ghostty", "tmux-256color", false),
            ProtocolType::Halfblocks
        );
        assert_eq!(
            protocol_for("iTerm.app", "screen-256color", false),
            ProtocolType::Halfblocks
        );
        assert_eq!(
            protocol_for("kitty", "xterm-kitty", true),
            ProtocolType::Halfblocks
        );
    }
}
