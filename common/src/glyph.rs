//! Icon-font glyphs shared by the WinRust programs: which icon font exists on this Windows, and drawing one glyph
//! centred on its ink (DrawText's DT_VCENTER centres the font's line box, which leaves icons a pixel or two off).
use std::sync::OnceLock;
use windows_sys::Win32::{
    Foundation::RECT,
    Graphics::Gdi::*,
};

const FLUENT: &str = "Segoe Fluent Icons";
const MDL2: &str = "Segoe MDL2 Assets";

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

/// Segoe Fluent Icons ships with Windows 11 only; Windows 10 (LTSC included) has Segoe MDL2 Assets with the same code points.
pub fn icon_face() -> &'static str {
    static FACE: OnceLock<&'static str> = OnceLock::new();
    FACE.get_or_init(|| if installed(FLUENT) { FLUENT } else { MDL2 })
}

/// Whether a font family is installed. GDI silently substitutes a missing face, so ask the DC which face it really selected.
pub fn installed(face: &str) -> bool {
    unsafe {
        let dc = CreateCompatibleDC(std::ptr::null_mut());
        if dc.is_null() {
            return false;
        }
        let f = CreateFontW(-16, 0, 0, 0, 400, 0, 0, 0, DEFAULT_CHARSET as u32, 0, 0, 0, 0, wide(face).as_ptr());
        let old = SelectObject(dc, f);
        let mut got = [0u16; 64];
        let n = GetTextFaceW(dc, got.len() as i32, got.as_mut_ptr());
        SelectObject(dc, old);
        DeleteObject(f);
        DeleteDC(dc);
        n > 0 && String::from_utf16_lossy(&got[..(n as usize).saturating_sub(1)]).eq_ignore_ascii_case(face)
    }
}

/// Draws one glyph of the font already selected into `dc`, its ink box centred in `r`. Falls back to DrawText for
/// anything GDI cannot measure (several characters, a missing glyph).
///
/// # Safety
/// `dc` must be a valid device context.
pub unsafe fn draw(dc: HDC, s: &str, r: RECT, color: u32) {
    unsafe {
        SetTextColor(dc, color);
        SetBkMode(dc, TRANSPARENT as i32);
        let w: Vec<u16> = s.encode_utf16().collect();
        let mut gm: GLYPHMETRICS = std::mem::zeroed();
        let ident = MAT2 { eM11: FIXED { fract: 0, value: 1 }, eM12: FIXED { fract: 0, value: 0 }, eM21: FIXED { fract: 0, value: 0 }, eM22: FIXED { fract: 0, value: 1 } };
        if w.len() == 1 && GetGlyphOutlineW(dc, w[0] as u32, GGO_METRICS, &mut gm, 0, std::ptr::null_mut(), &ident) != GDI_ERROR as u32 && gm.gmBlackBoxX > 0 {
            let (cx2, cy2) = (r.left + r.right, r.top + r.bottom); // twice the centre: keeps the halves exact
            let x = (cx2 - gm.gmBlackBoxX as i32) / 2 - gm.gmptGlyphOrigin.x;
            let base = (cy2 - gm.gmBlackBoxY as i32) / 2 + gm.gmptGlyphOrigin.y;
            let align = SetTextAlign(dc, TA_LEFT | TA_BASELINE);
            TextOutW(dc, x, base, w.as_ptr(), 1);
            SetTextAlign(dc, align);
        } else {
            let mut rc = r;
            let mut w = w;
            w.push(0);
            DrawTextW(dc, w.as_ptr(), -1, &mut rc, DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn some_icon_font_is_picked() {
        assert!([FLUENT, MDL2].contains(&icon_face()));
    }
}
