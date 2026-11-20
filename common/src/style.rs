//! The UI style shared by the WinRust programs: %ProgramData%\WinRust\style.txt holds its name (the topbar gear menu writes it).
//! Programs re-read it at most once a second and repaint at once on the broadcast `set_style` sends (`msg()`).
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU8, AtomicU32, AtomicU64, Ordering::Relaxed},
};
use windows_sys::Win32::UI::WindowsAndMessaging::{HWND_BROADCAST, PostMessageW, RegisterWindowMessageW};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Style {
    Normal,
    Terminal, // monospace, green on black, square corners
    Dock,     // the bar is a floating pill in the middle
    Tiling,   // a thin status strip at the top, square corners, windows side by side
}

pub const ALL: [Style; 4] = [Style::Normal, Style::Terminal, Style::Dock, Style::Tiling];

impl Style {
    pub fn name(self) -> &'static str {
        match self {
            Style::Normal => "normal",
            Style::Terminal => "terminal",
            Style::Dock => "dock",
            Style::Tiling => "tiling",
        }
    }
    /// Unknown or empty text (no file yet, a hand edit) is the normal style.
    pub fn parse(s: &str) -> Style {
        ALL.into_iter().find(|v| v.name().eq_ignore_ascii_case(s.trim())).unwrap_or(Style::Normal)
    }
    /// Square corners everywhere.
    pub fn square(self) -> bool {
        matches!(self, Style::Terminal | Style::Tiling)
    }
    pub fn mono(self) -> bool {
        matches!(self, Style::Terminal | Style::Tiling)
    }
}

static CUR: AtomicU8 = AtomicU8::new(u8::MAX); // index into ALL; MAX = not read yet
static READ_AT: AtomicU64 = AtomicU64::new(0); // when the file was last read, ms since the epoch

fn file() -> Option<PathBuf> {
    std::env::var_os("ProgramData").map(|p| PathBuf::from(p).join("WinRust").join("style.txt"))
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

fn store(s: Style) {
    CUR.store(ALL.iter().position(|&v| v == s).unwrap_or(0) as u8, Relaxed);
    READ_AT.store(now_ms(), Relaxed);
}

/// Reads the file again now (on the "style changed" broadcast).
pub fn reload() {
    store(Style::parse(&file().and_then(|f| std::fs::read_to_string(f).ok()).unwrap_or_default()));
}

/// The current style. The file is read again at most once a second, so a program that missed the broadcast still follows.
pub fn style() -> Style {
    if CUR.load(Relaxed) == u8::MAX || now_ms().saturating_sub(READ_AT.load(Relaxed)) > 1000 {
        reload();
    }
    ALL[CUR.load(Relaxed) as usize % ALL.len()]
}

/// Switches this process, writes the choice and tells the other WinRust windows to repaint in it.
pub fn set_style(s: Style) {
    store(s);
    if let Some(f) = file() {
        let _ = std::fs::create_dir_all(f.parent().unwrap());
        let _ = std::fs::write(f, s.name());
    }
    unsafe { PostMessageW(HWND_BROADCAST, msg(), 0, 0) };
}

/// The broadcast message a window gets when the style changed: call `reload()` and repaint.
pub fn msg() -> u32 {
    static M: AtomicU32 = AtomicU32::new(0);
    match M.load(Relaxed) {
        0 => {
            let n: Vec<u16> = "WinRustStyleChanged\0".encode_utf16().collect();
            let m = unsafe { RegisterWindowMessageW(n.as_ptr()) };
            M.store(m, Relaxed);
            m
        }
        m => m,
    }
}

/// Colours of one style, as COLORREF (0x00BBGGRR) like GDI wants them.
pub struct Pal {
    pub bg: u32,
    pub line: u32,
    pub hover: u32,
    pub active: u32,
    pub accent: u32,
    pub text: u32,
    pub dim: u32,
}

pub const fn rgb(r: u8, g: u8, b: u8) -> u32 {
    r as u32 | (g as u32) << 8 | (b as u32) << 16
}

/// In the order of `ALL`.
pub const PALS: [Pal; 4] = [
    // normal: Windows 11 blue
    Pal { bg: rgb(24, 30, 44), line: rgb(44, 52, 72), hover: rgb(40, 48, 68), active: rgb(52, 61, 84), accent: rgb(76, 194, 255), text: rgb(255, 255, 255), dim: rgb(150, 158, 175) },
    // terminal: green on black
    Pal { bg: rgb(10, 15, 10), line: rgb(40, 72, 40), hover: rgb(20, 40, 20), active: rgb(28, 56, 28), accent: rgb(91, 215, 91), text: rgb(207, 232, 207), dim: rgb(100, 145, 100) },
    // dock: graphite with violet
    Pal { bg: rgb(27, 29, 34), line: rgb(50, 53, 62), hover: rgb(42, 45, 53), active: rgb(52, 55, 66), accent: rgb(167, 139, 250), text: rgb(255, 255, 255), dim: rgb(154, 160, 170) },
    // tiling: dark with amber
    Pal { bg: rgb(17, 19, 23), line: rgb(40, 44, 52), hover: rgb(30, 34, 41), active: rgb(38, 43, 52), accent: rgb(229, 192, 123), text: rgb(215, 218, 224), dim: rgb(110, 117, 130) },
];

pub fn pal() -> &'static Pal {
    &PALS[ALL.iter().position(|&s| s == style()).unwrap_or(0)]
}

/// The UI font of the current style.
pub fn ui_face() -> &'static str {
    if style().mono() { "Consolas" } else { sans_face() }
}

/// The proportional UI font: SF Pro Display when the user has installed it (it is not ours to ship), else Segoe UI.
pub fn sans_face() -> &'static str {
    static FACE: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();
    FACE.get_or_init(|| if crate::glyph::installed("SF Pro Display") { "SF Pro Display" } else { "Segoe UI" })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip_and_junk_is_normal() {
        for s in ALL {
            assert_eq!(Style::parse(s.name()), s);
        }
        assert_eq!(Style::parse(" Tiling\r\n"), Style::Tiling);
        assert_eq!(Style::parse(""), Style::Normal);
        assert_eq!(Style::parse("neon"), Style::Normal);
    }
}
