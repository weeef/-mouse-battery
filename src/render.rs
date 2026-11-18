//! Pixel drawing: the taskbar widget ("<mouse outline> 93%", Windows 11 tray style)
//! and the small pixel-art tray icon. Pixels are 0xAARRGGBB, top-down rows.

use crate::state::{Entry, LOW_BATTERY};
use std::sync::OnceLock;

pub type Rgb = (u8, u8, u8);

pub struct Canvas {
    pub w: usize,
    pub h: usize,
    pub px: Vec<u32>,
}

impl Canvas {
    pub fn new(w: usize, h: usize, fill: u32) -> Canvas {
        Canvas { w, h, px: vec![fill; w * h] }
    }

    fn blend(&mut self, x: i32, y: i32, c: Rgb, a: f32) {
        if x < 0 || y < 0 || x as usize >= self.w || y as usize >= self.h || a <= 0.0 {
            return;
        }
        let a = a.min(1.0);
        let i = y as usize * self.w + x as usize;
        let p = self.px[i];
        let mix = |src: u8, dst: u32| (src as f32 * a + dst as f32 * (1.0 - a)).round() as u32;
        let r = mix(c.0, (p >> 16) & 0xFF);
        let g = mix(c.1, (p >> 8) & 0xFF);
        let b = mix(c.2, p & 0xFF);
        self.px[i] = 0xFF00_0000 | (r << 16) | (g << 8) | b;
    }
}

pub fn rgb_u32(c: Rgb) -> u32 {
    0xFF00_0000 | ((c.0 as u32) << 16) | ((c.1 as u32) << 8) | c.2 as u32
}

// ------------------------------------------------------------------ text

fn font() -> Option<&'static fontdue::Font> {
    static FONT: OnceLock<Option<fontdue::Font>> = OnceLock::new();
    FONT.get_or_init(|| {
        let dir = std::path::PathBuf::from(std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into())).join("Fonts");
        ["seguisb.ttf", "segoeui.ttf", "arial.ttf"].iter().find_map(|name| {
            let bytes = std::fs::read(dir.join(name)).ok()?;
            fontdue::Font::from_bytes(bytes, fontdue::FontSettings::default()).ok()
        })
    })
    .as_ref()
}

struct Glyph {
    x: i32, // ink left relative to pen origin
    top: i32, // ink top relative to baseline (positive = above)
    w: usize,
    h: usize,
    bmp: Vec<u8>,
}

struct Text {
    glyphs: Vec<(f32, Glyph)>, // pen x, glyph
    ink_left: i32,
    ink_right: i32,
    ink_top: i32,
    ink_bottom: i32,
}

fn layout(text: &str, px: f32) -> Option<Text> {
    let font = font()?;
    let mut pen = 0.0f32;
    let mut out = Text { glyphs: vec![], ink_left: i32::MAX, ink_right: i32::MIN, ink_top: i32::MIN, ink_bottom: i32::MAX };
    for ch in text.chars() {
        let (m, bmp) = font.rasterize(ch, px);
        let g = Glyph { x: m.xmin, top: m.ymin + m.height as i32, w: m.width, h: m.height, bmp };
        let left = pen.round() as i32 + g.x;
        out.ink_left = out.ink_left.min(left);
        out.ink_right = out.ink_right.max(left + g.w as i32);
        out.ink_top = out.ink_top.max(g.top);
        out.ink_bottom = out.ink_bottom.min(m.ymin);
        out.glyphs.push((pen, g));
        pen += m.advance_width;
    }
    Some(out)
}

fn draw_text(c: &mut Canvas, t: &Text, x: i32, baseline: i32, color: Rgb) {
    for (pen, g) in &t.glyphs {
        let gx = pen.round() as i32 + g.x + x;
        let gy = baseline - g.top;
        for row in 0..g.h {
            for col in 0..g.w {
                let a = g.bmp[row * g.w + col] as f32 / 255.0;
                c.blend(gx + col as i32, gy + row as i32, color, a);
            }
        }
    }
}

// ------------------------------------------------------------------ taskbar widget

fn widget_color(e: &Entry, bg: Rgb) -> Rgb {
    let light = 0.299 * bg.0 as f32 + 0.587 * bg.1 as f32 + 0.114 * bg.2 as f32 > 140.0;
    let pick = |dark: Rgb, on_light: Rgb| if light { on_light } else { dark };
    match e.level {
        None => pick((160, 160, 160), (110, 110, 110)),
        _ if e.asleep => pick((160, 160, 160), (110, 110, 110)),
        _ if e.charging => pick((96, 180, 255), (0, 103, 192)),
        Some(l) if l <= LOW_BATTERY => pick((255, 99, 99), (196, 43, 28)),
        _ => pick((255, 255, 255), (20, 20, 20)),
    }
}

fn coverage(dist: f32, half_width: f32) -> f32 {
    (half_width - dist + 0.5).clamp(0.0, 1.0)
}

/// Mouse outline: a stadium (rounded rectangle with radius = width/2) plus the button split line.
fn draw_mouse(c: &mut Canvas, x0: f32, y0: f32, w: f32, h: f32, lw: f32, color: Rgb) {
    let r = w / 2.0;
    let cx = x0 + r;
    let (cy0, cy1) = (y0 + r, y0 + h - r);
    let line_end = y0 + h * 0.36;
    let pad = lw.ceil() as i32 + 1;
    for py in (y0 as i32 - pad)..=((y0 + h) as i32 + pad) {
        for px in (x0 as i32 - pad)..=((x0 + w) as i32 + pad) {
            let (fx, fy) = (px as f32 + 0.5, py as f32 + 0.5);
            let outline = ((fx - cx).hypot(fy - fy.clamp(cy0, cy1)) - r).abs();
            let line = (fx - cx).hypot(fy - fy.clamp(y0, line_end));
            let a = coverage(outline, lw / 2.0).max(coverage(line, lw / 2.0));
            c.blend(px, py, color, a);
        }
    }
}

pub fn render_widget(entries: &[Entry], bg: Rgb, scale: f32, height: usize) -> Canvas {
    let none = [Entry { level: None, charging: false, asleep: false }];
    let entries = if entries.is_empty() { &none[..] } else { entries };
    let s = scale;
    let (pad, icon_w, icon_h, gap, between, lw) = (6.0 * s, 11.0 * s, 16.0 * s, 5.0 * s, 12.0 * s, 1.5 * s);
    let font_px = 14.0 * s;

    let items: Vec<(Option<Text>, Rgb)> = entries
        .iter()
        .map(|e| {
            let text = match e.level {
                Some(l) => format!("{l}%"),
                None => "--%".into(),
            };
            (layout(&text, font_px), widget_color(e, bg))
        })
        .collect();
    let text_w = |t: &Option<Text>| t.as_ref().map_or(0.0, |t| (t.ink_right - t.ink_left) as f32);
    let width = 2.0 * pad
        + items.iter().map(|(t, _)| icon_w + gap + text_w(t)).sum::<f32>()
        + between * (items.len() as f32 - 1.0);

    let mut c = Canvas::new(width.round() as usize, height, rgb_u32(bg));
    let mut x = pad;
    for (text, color) in &items {
        draw_mouse(&mut c, x, (height as f32 - icon_h) / 2.0, icon_w, icon_h, lw, *color);
        x += icon_w + gap;
        if let Some(t) = text {
            let ink_h = t.ink_top - t.ink_bottom;
            let baseline = ((height as i32 - ink_h) as f32 / 2.0).round() as i32 + t.ink_top;
            draw_text(&mut c, t, x.round() as i32 - t.ink_left, baseline, *color);
        }
        x += text_w(text) + between;
    }
    c
}

// ------------------------------------------------------------------ tray icon (16x16 pixel art)

const MOUSE: [&str; 16] = [
    "...#####...", ".#########.", ".#########.", "###########", "###########", "###########",
    "###########", "###########", "###########", "###########", "###########", "###########",
    "###########", ".#########.", ".#########.", "...#####...",
];
const MOUSE_X: i32 = 2;
const SPLIT: i32 = 5; // row of the line between buttons and palm

fn digit(ch: char) -> &'static str {
    // 3x5 pixel digits; "1" is 1 pixel wide
    match ch {
        '0' => "111101101101111",
        '1' => "11111",
        '2' => "111001111100111",
        '3' => "111001111001111",
        '4' => "101101111001001",
        '5' => "111100111001111",
        '6' => "111100111101111",
        '7' => "111001001001001",
        '8' => "111101111101111",
        '9' => "111101111001111",
        _ => "000000111000000",
    }
}

fn tray_color(e: Option<&Entry>) -> Rgb {
    match e {
        Some(Entry { level: Some(l), charging, asleep: false }) => {
            if *charging {
                (80, 170, 255)
            } else if *l <= LOW_BATTERY {
                (240, 70, 70)
            } else if *l <= 35 {
                (245, 190, 50)
            } else {
                (90, 210, 110)
            }
        }
        _ => (150, 150, 150),
    }
}

/// Square tray icon of `size` pixels (16 at 100% scaling), transparent background.
pub fn render_tray_icon(e: Option<&Entry>, size: usize) -> Canvas {
    let mut small = Canvas::new(16, 16, 0);
    let color = rgb_u32(tray_color(e));
    let body = |x: i32, y: i32| {
        (0..16).contains(&y) && (0..11).contains(&x) && MOUSE[y as usize].as_bytes()[x as usize] == b'#'
    };
    let put = |c: &mut Canvas, x: i32, y: i32, v: u32| {
        if (0..16).contains(&x) && (0..16).contains(&y) {
            c.px[(y * 16 + x) as usize] = v;
        }
    };
    for y in 0..16 {
        for x in 0..11 {
            if body(x, y) && ![(1, 0), (-1, 0), (0, 1), (0, -1)].iter().all(|(dx, dy)| body(x + dx, y + dy)) {
                put(&mut small, MOUSE_X + x, y, color);
            }
        }
    }
    for x in 1..10 {
        put(&mut small, MOUSE_X + x, SPLIT, color);
    }
    for y in 0..SPLIT {
        put(&mut small, MOUSE_X + 5, y, color);
    }

    let text = match e.and_then(|e| e.level) {
        Some(l) => l.to_string(),
        None => "--".into(),
    };
    let widths: Vec<i32> = text.chars().map(|c| digit(c).len() as i32 / 5).collect();
    let total = widths.iter().sum::<i32>() + widths.len() as i32 - 1;
    let mut x = MOUSE_X + (11 - total + 1) / 2;
    for (ch, w) in text.chars().zip(&widths) {
        for (i, on) in digit(ch).bytes().enumerate() {
            if on == b'1' {
                put(&mut small, x + i as i32 % w, 8 + i as i32 / w, 0xFFFF_FFFF);
            }
        }
        x += w + 1;
    }

    // bigger tray slot at >100% scaling: scale up keeping hard pixel edges
    let mut out = Canvas::new(size, size, 0);
    for y in 0..size {
        for x in 0..size {
            out.px[y * size + x] = small.px[(y * 16 / size) * 16 + x * 16 / size];
        }
    }
    out
}
