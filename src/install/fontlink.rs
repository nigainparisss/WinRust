//! SF Pro Display has no Georgian letters. When the user has installed it, GDI font linking takes the missing letters
//! from Microsoft Sans Serif (then Segoe UI) in every WinRust window. The setting is system-wide (HKLM) and Windows reads it at sign-in.
use crate::*;

const LINK: &str = r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\FontLink\SystemLink";
const FACE: &str = "SF Pro Display";
/// Whatever link list was there before ours, saved once and put back when the last plugin goes.
const STATE: &str = r"HKLM\SOFTWARE\plugin-launcher\fontlink";
const OURS: &str = r"MICROSS.TTF,Microsoft Sans Serif\0SEGOEUI.TTF,Segoe UI";

fn ours() -> bool {
    reg_str(LINK, FACE).is_some_and(|v| v.starts_with(OURS))
}

pub(crate) fn fontlink_install() -> Result<(), String> {
    if !i18n::glyph::installed(FACE) || ours() {
        return Ok(());
    }
    let old = reg_str(LINK, FACE).filter(|o| !o.is_empty());
    if reg_dword(STATE, "Saved").is_none() {
        if let Some(o) = &old {
            run("reg", &["add", STATE, "/v", "Prev", "/t", "REG_SZ", "/d", o, "/f"])?;
        }
        run("reg", &["add", STATE, "/v", "Saved", "/t", "REG_DWORD", "/d", "1", "/f"])?;
    }
    let new = old.map_or(OURS.to_string(), |o| format!(r"{OURS}\0{o}"));
    run("reg", &["add", LINK, "/v", FACE, "/t", "REG_MULTI_SZ", "/d", &new, "/f"])
}

pub(crate) fn fontlink_uninstall() -> Result<(), String> {
    if reg_dword(STATE, "Saved").is_none() {
        return Ok(());
    }
    if ours() {
        match reg_str(STATE, "Prev") {
            Some(p) => run("reg", &["add", LINK, "/v", FACE, "/t", "REG_MULTI_SZ", "/d", &p, "/f"])?,
            None => run("reg", &["delete", LINK, "/v", FACE, "/f"])?,
        }
    }
    run("reg", &["delete", STATE, "/f"])
}
