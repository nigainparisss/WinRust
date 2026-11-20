#![windows_subsystem = "windows"]
// wiztree [X:]  Disk usage treemap on the NTFS MFT (the same reader as rs-du, from winrust-common).
// Click a folder: go in. Right click / Backspace: up. Click a file: show it in Explorer. Tab: next drive. F5: rescan. Esc: quit.
// Needs admin (raw volume); starts itself elevated (UAC) when it is not.
// shortcuts: one level of the tree at a time, the 300 biggest items; a folder's own index size is not counted (see common/src/mft.rs).
use winrust_common::mft::*;
use std::{cell::RefCell, collections::HashMap, ptr::{null, null_mut}};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives},
    System::LibraryLoader::GetModuleHandleW,
    UI::{HiDpi::GetDpiForSystem, Input::KeyboardAndMouse::*, Shell::{IsUserAnAdmin, ShellExecuteW}, WindowsAndMessaging::*},
};

const WM_DONE: u32 = WM_APP + 1; // lparam = Box<Result<Tree, String>>
const DRIVE_FIXED: u32 = 3;
const MAX_CELLS: usize = 300;
const BG: u32 = rgb(22, 19, 18);
const DIM: u32 = rgb(166, 152, 145);
const ACCENT: u32 = rgb(222, 98, 42);

const fn rgb(r: u8, g: u8, b: u8) -> u32 {
    r as u32 | (g as u32) << 8 | (b as u32) << 16
}

struct Tree {
    recs: HashMap<u32, Rec>,
    tot: HashMap<u32, u64>,
    kids: HashMap<u32, Vec<u32>>,
}

fn build(recs: HashMap<u32, Rec>) -> Tree {
    let tot = totals(&recs, false);
    let mut kids: HashMap<u32, Vec<u32>> = HashMap::new();
    for (&id, r) in &recs {
        if id != ROOT && tot.contains_key(&id) {
            kids.entry(r.parent).or_default().push(id);
        }
    }
    for v in kids.values_mut() {
        v.sort_by_key(|id| std::cmp::Reverse(tot[id]));
    }
    Tree { recs, tot, kids }
}

#[derive(Default)]
struct App {
    tree: Option<Tree>,
    drive: char,
    cur: u32,
    status: String,
    cells: Vec<(RECT, u32)>,
    hot: Option<usize>,
    dpi: i32,
    font: isize,
}
thread_local! { static APP: RefCell<App> = RefCell::new(App { drive: 'C', cur: ROOT, dpi: 96, ..Default::default() }) }

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

/// Squarified treemap: `items` (id, size) biggest first, laid out in the rectangle (x, y, w, h).
fn squarify(items: &[(u32, u64)], (mut x, mut y, mut w, mut h): (f64, f64, f64, f64)) -> Vec<(u32, [f64; 4])> {
    let total: f64 = items.iter().map(|i| i.1 as f64).sum();
    let mut out = vec![];
    if total <= 0.0 || w <= 0.0 || h <= 0.0 {
        return out;
    }
    let scale = w * h / total;
    let area = |k: usize| items[k].1 as f64 * scale;
    let mut i = 0;
    while i < items.len() {
        let side = w.min(h);
        let worst = |s: f64, mx: f64, mn: f64| (side * side * mx / (s * s)).max(s * s / (side * side * mn));
        let (mut j, mut s, mut mx, mut mn) = (i + 1, area(i), area(i), area(i));
        let mut cur = worst(s, mx, mn);
        while j < items.len() {
            let a = area(j);
            let next = worst(s + a, mx.max(a), mn.min(a));
            if next > cur {
                break;
            }
            (s, mx, mn, cur, j) = (s + a, mx.max(a), mn.min(a), next, j + 1);
        }
        if w >= h {
            // a column on the left
            let (cw, mut yy) = (s / h, y);
            for k in i..j {
                let ch = area(k) / cw;
                out.push((items[k].0, [x, yy, x + cw, yy + ch]));
                yy += ch;
            }
            x += cw;
            w -= cw;
        } else {
            // a row on top
            let (rh, mut xx) = (s / w, x);
            for k in i..j {
                let cw = area(k) / rh;
                out.push((items[k].0, [xx, y, xx + cw, y + rh]));
                xx += cw;
            }
            y += rh;
            h -= rh;
        }
        i = j;
    }
    out
}

/// Stable colour per extension / folder name (folders darker than files).
fn color(name: &str, dir: bool) -> u32 {
    let key = if dir { name } else { name.rsplit_once('.').map_or("", |e| e.1) };
    let hue = (key.bytes().fold(7u32, |a, b| a.wrapping_mul(31).wrapping_add(b.to_ascii_lowercase() as u32)) % 360) as f64;
    let (s, v) = if dir { (0.35, 0.42) } else { (0.5, 0.72) };
    let f = |n: f64| {
        let k = (n + hue / 60.0) % 6.0;
        ((v - v * s * k.min(4.0 - k).clamp(0.0, 1.0)) * 255.0) as u8
    };
    rgb(f(5.0), f(3.0), f(1.0))
}

fn fixed_drives() -> Vec<char> {
    let mask = unsafe { GetLogicalDrives() };
    ('A'..='Z').filter(|c| mask & (1 << (*c as u8 - b'A')) != 0 && unsafe { GetDriveTypeW(wide(&format!("{c}:\\")).as_ptr()) } == DRIVE_FIXED).collect()
}

fn rescan(hwnd: HWND, drive: char) {
    APP.with_borrow_mut(|a| {
        (a.tree, a.drive, a.cur, a.hot, a.cells) = (None, drive, ROOT, None, vec![]);
        a.status = format!("Reading the MFT of {drive}: ...");
    });
    unsafe { InvalidateRect(hwnd, null(), 0) };
    let h = hwnd as isize;
    std::thread::spawn(move || {
        let r: Result<Tree, String> = scan(drive).map(build);
        unsafe { PostMessageW(h as HWND, WM_DONE, 0, Box::into_raw(Box::new(r)) as isize) };
    });
}

unsafe fn paint(hwnd: HWND) {
    unsafe {
        let mut ps: PAINTSTRUCT = std::mem::zeroed();
        let dc = BeginPaint(hwnd, &mut ps);
        let mut rc: RECT = std::mem::zeroed();
        GetClientRect(hwnd, &mut rc);
        let (w, h) = (rc.right, rc.bottom);
        let mem = CreateCompatibleDC(dc);
        let bmp = CreateCompatibleBitmap(dc, w, h);
        let old = SelectObject(mem, bmp);
        APP.with_borrow_mut(|a| {
            let k = a.dpi as f64 / 96.0;
            let (top, bot) = ((44.0 * k) as i32, (30.0 * k) as i32);
            let fill = |r: &RECT, c: u32| {
                let b = CreateSolidBrush(c);
                FillRect(mem, r, b);
                DeleteObject(b as _);
            };
            fill(&rc, BG);
            SelectObject(mem, a.font as _);
            SetBkMode(mem, TRANSPARENT as _);
            let text = |s: &str, mut r: RECT, c: u32, fmt: u32| {
                SetTextColor(mem, c);
                let t = wide(s);
                DrawTextW(mem, t.as_ptr(), -1, &mut r, fmt);
            };
            let one = DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX;
            a.cells.clear();
            let mut head = format!("{}:", a.drive);
            let mut foot = a.status.clone();
            if let Some(t) = &a.tree {
                head = path_of(&t.recs, a.cur, a.drive).0;
                let items: Vec<(u32, u64)> = t.kids.get(&a.cur).map_or(vec![], |v| v.iter().take(MAX_CELLS).map(|&id| (id, t.tot[&id])).collect());
                let pad = 8.0 * k;
                for (id, r) in squarify(&items, (pad, top as f64, w as f64 - 2.0 * pad, (h - top - bot) as f64 - pad)) {
                    a.cells.push((RECT { left: r[0] as i32, top: r[1] as i32, right: r[2] as i32, bottom: r[3] as i32 }, id));
                }
                for (n, (c, id)) in a.cells.iter().enumerate() {
                    let rec = &t.recs[id];
                    let inner = RECT { left: c.left + 1, top: c.top + 1, right: c.right - 1, bottom: c.bottom - 1 };
                    fill(&inner, color(&rec.name, rec.dir));
                    if a.hot == Some(n) {
                        let b = CreateSolidBrush(rgb(255, 255, 255));
                        FrameRect(mem, &inner, b);
                        DeleteObject(b as _);
                    }
                    if inner.right - inner.left > (70.0 * k) as i32 && inner.bottom - inner.top > (34.0 * k) as i32 {
                        let r = RECT { left: inner.left + 6, top: inner.top + 4, right: inner.right - 4, bottom: inner.bottom - 2 };
                        text(&format!("{}\n{}", rec.name, human(t.tot[id])), r, rgb(255, 255, 255), DT_LEFT | DT_TOP | DT_WORDBREAK | DT_END_ELLIPSIS | DT_NOPREFIX);
                    }
                }
                let size = if a.cur == ROOT { t.tot.values().max().copied().unwrap_or(0) } else { t.tot.get(&a.cur).copied().unwrap_or(0) };
                head = format!("{head}    {}", human(size));
                foot = match a.hot.and_then(|n| a.cells.get(n)) {
                    Some((_, id)) => format!("{}    {}", path_of(&t.recs, *id, a.drive).0, human(t.tot[id])),
                    None => "Click: open folder / show file    Right click, Backspace: up    Tab: next drive    F5: rescan".into(),
                };
            }
            text(&head, RECT { left: (12.0 * k) as i32, top: 0, right: w - 12, bottom: top }, ACCENT, one);
            text(&foot, RECT { left: (12.0 * k) as i32, top: h - bot, right: w - 12, bottom: h }, DIM, one);
        });
        BitBlt(dc, 0, 0, w, h, mem, 0, 0, SRCCOPY);
        SelectObject(mem, old);
        DeleteObject(bmp as _);
        DeleteDC(mem);
        EndPaint(hwnd, &ps);
    }
}

fn up(hwnd: HWND) {
    APP.with_borrow_mut(|a| {
        if let (Some(t), true) = (&a.tree, a.cur != ROOT) {
            a.cur = t.recs.get(&a.cur).map_or(ROOT, |r| r.parent);
            a.hot = None;
        }
    });
    unsafe { InvalidateRect(hwnd, null(), 0) };
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_CREATE => {
                let dpi = GetDpiForSystem() as i32;
                let f = CreateFontW(-(15 * dpi / 96), 0, 0, 0, 400, 0, 0, 0, 1, 0, 0, 5, 0, wide(winrust_common::style::sans_face()).as_ptr());
                APP.with_borrow_mut(|a| (a.dpi, a.font) = (dpi, f as isize));
                0
            }
            WM_PAINT => {
                paint(hwnd);
                0
            }
            WM_ERASEBKGND => 1,
            WM_DONE => {
                match *Box::from_raw(lp as *mut Result<Tree, String>) {
                    Ok(t) => APP.with_borrow_mut(|a| {
                        a.status = String::new();
                        a.tree = Some(t);
                    }),
                    Err(e) => APP.with_borrow_mut(|a| a.status = e),
                }
                InvalidateRect(hwnd, null(), 0);
                0
            }
            WM_MOUSEMOVE => {
                let (x, y) = ((lp & 0xFFFF) as i16 as i32, (lp >> 16) as i16 as i32);
                let changed = APP.with_borrow_mut(|a| {
                    let n = a.cells.iter().position(|(r, _)| x >= r.left && x < r.right && y >= r.top && y < r.bottom);
                    std::mem::replace(&mut a.hot, n) != n
                });
                if changed {
                    InvalidateRect(hwnd, null(), 0);
                }
                0
            }
            WM_LBUTTONDOWN => {
                let act = APP.with_borrow_mut(|a| {
                    let id = a.hot.and_then(|n| a.cells.get(n))?.1;
                    let t = a.tree.as_ref()?;
                    if t.recs[&id].dir {
                        let p = None;
                        (a.cur, a.hot) = (id, None);
                        p
                    } else {
                        Some(path_of(&t.recs, id, a.drive).0)
                    }
                });
                if let Some(p) = act {
                    // show the file in Explorer (delete it from there)
                    ShellExecuteW(null_mut(), wide("open").as_ptr(), wide("explorer.exe").as_ptr(), wide(&format!("/select,\"{p}\"")).as_ptr(), null(), SW_SHOWNORMAL);
                }
                InvalidateRect(hwnd, null(), 0);
                0
            }
            WM_RBUTTONDOWN => {
                up(hwnd);
                0
            }
            WM_KEYDOWN => {
                match wp as u16 {
                    VK_ESCAPE => {
                        DestroyWindow(hwnd);
                    }
                    VK_BACK => up(hwnd),
                    VK_F5 => rescan(hwnd, APP.with_borrow(|a| a.drive)),
                    VK_TAB => {
                        let (d, all) = (APP.with_borrow(|a| a.drive), fixed_drives());
                        if let Some(n) = all.iter().position(|&c| c == d) {
                            rescan(hwnd, all[(n + 1) % all.len()]);
                        }
                    }
                    _ => {}
                }
                0
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                0
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

fn main() {
    unsafe {
        let drive = std::env::args().nth(1).and_then(|a| a.chars().next()).filter(char::is_ascii_alphabetic).map_or('C', |c| c.to_ascii_uppercase());
        if IsUserAnAdmin() == 0 {
            // raw volume access needs it: ask once through UAC and let the elevated copy do the work
            let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default();
            ShellExecuteW(null_mut(), wide("runas").as_ptr(), wide(&exe).as_ptr(), wide(&format!("{drive}:")).as_ptr(), null(), SW_SHOWNORMAL);
            return;
        }
        SetProcessDPIAware();
        let (inst, class) = (GetModuleHandleW(null()), wide("WinRustDisk"));
        let wc = WNDCLASSW { style: 0, lpfnWndProc: Some(proc), cbClsExtra: 0, cbWndExtra: 0, hInstance: inst, hIcon: null_mut(), hCursor: LoadCursorW(null_mut(), IDC_ARROW), hbrBackground: null_mut(), lpszMenuName: null(), lpszClassName: class.as_ptr() };
        RegisterClassW(&wc);
        let hwnd = CreateWindowExW(0, class.as_ptr(), wide("WinRust Disk").as_ptr(), WS_OVERLAPPEDWINDOW | WS_VISIBLE, CW_USEDEFAULT, CW_USEDEFAULT, 1280, 800, null_mut(), null_mut(), inst, null());
        rescan(hwnd, drive);
        let mut m: MSG = std::mem::zeroed();
        while GetMessageW(&mut m, null_mut(), 0, 0) > 0 {
            TranslateMessage(&m);
            DispatchMessageW(&m);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn treemap_fills_the_rect_and_keeps_size_ratios() {
        let items = [(1, 600), (2, 300), (3, 200), (4, 100), (5, 100)];
        let cells = squarify(&items, (0.0, 0.0, 400.0, 300.0));
        assert_eq!(cells.len(), 5);
        let a = |id: u32| {
            let r = cells.iter().find(|c| c.0 == id).unwrap().1;
            (r[2] - r[0]) * (r[3] - r[1])
        };
        let total: f64 = (1..=5).map(a).sum();
        assert!((total - 120_000.0).abs() < 1.0, "area {total}");
        assert!((a(1) / a(2) - 2.0).abs() < 1e-6);
    }
}
