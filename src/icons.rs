//! Pictures: the pixel cloud of the tray and window icon, with a sign for
//! the state, and the suite's retro file icons (NIRUICONS) for the Drive.
use crate::canvas::{Canvas, Image, Rect};
use crate::theme::{blend, Palette};
use std::collections::HashMap;
use std::rc::Rc;

/// What the tray icon says.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Sign {
    /// Everything sent.
    Done,
    /// Sending or fetching.
    Busy,
    Paused,
    Error,
    /// Signed out or no CLI: a dash.
    Off,
}

/// The cloud on a 16x16 grid: `#` body, `.` empty.
const CLOUD: [&str; 16] = [
    "................",
    "................",
    "................",
    "......####......",
    ".....######.....",
    "....########....",
    "..###########...",
    ".##############.",
    "################",
    "################",
    "################",
    "################",
    ".##############.",
    "..############..",
    "................",
    "................",
];

/// The signs, 7x6, drawn out of the cloud's body in the ground colour.
fn sign_rows(s: Sign) -> [&'static str; 6] {
    match s {
        Sign::Done => [
            "......#", ".....##", "#...##.", "##.##..", ".###...", "..#....",
        ],
        Sign::Busy => [
            "...#...", "..###..", ".#####.", "...#...", "...#...", "...#...",
        ],
        Sign::Paused => [
            ".##.##.", ".##.##.", ".##.##.", ".##.##.", ".##.##.", ".##.##.",
        ],
        Sign::Error => [
            "...#...", "...#...", "...#...", "...#...", ".......", "...#...",
        ],
        Sign::Off => [
            ".......", ".......", "#######", "#######", ".......", ".......",
        ],
    }
}

/// The cloud as 16x16 cells: 0 empty, 1 body, 3 sign.
pub fn cells(sign: Sign) -> [[u8; 16]; 16] {
    let mut g = [[0u8; 16]; 16];
    for (y, row) in CLOUD.iter().enumerate() {
        for (x, c) in row.bytes().enumerate() {
            if c == b'#' {
                g[y][x] = 1;
            }
        }
    }
    for (y, row) in sign_rows(sign).iter().enumerate() {
        for (x, c) in row.bytes().enumerate() {
            if c == b'#' {
                g[6 + y][5 + x] = 3;
            }
        }
    }
    g
}

/// The cloud drawn into `r` of a canvas, whole cells only, centred.
pub fn draw_cloud(cv: &mut Canvas, r: Rect, sign: Sign, p: &Palette, ground: u32) {
    let u = (r.w.min(r.h) / 16).max(1);
    let (ox, oy) = (r.x + (r.w - 16 * u) / 2, r.y + (r.h - 16 * u) / 2);
    let body = match sign {
        Sign::Error => p.error,
        // Signed out and paused: the text colour, a little dimmed, so the
        // cloud reads clearly in the bar without looking busy or done.
        Sign::Paused | Sign::Off => blend(p.foreground, p.muted, 0.5),
        _ => p.mark(),
    };
    let g = cells(sign);
    for (y, row) in g.iter().enumerate() {
        for (x, c) in row.iter().enumerate() {
            let col = match c {
                1 => body,
                3 => ground,
                _ => continue,
            };
            cv.fill(Rect::new(ox + x as i32 * u, oy + y as i32 * u, u, u), col);
        }
    }
}

/// ARGB window icons, 16, 32 and 48 pixels.
pub fn window_icons(p: &Palette) -> Vec<(u32, u32, Vec<u32>)> {
    [16u32, 32, 48]
        .iter()
        .map(|&s| {
            let mut cv = Canvas::new(s as i32, s as i32);
            let ground = 0x00ff00ff;
            cv.fill(Rect::new(0, 0, s as i32, s as i32), ground);
            draw_cloud(
                &mut cv,
                Rect::new(0, 0, s as i32, s as i32),
                Sign::Done,
                p,
                p.background,
            );
            let argb = cv
                .px
                .iter()
                .map(|v| if *v == ground { 0 } else { 0xff00_0000 | v })
                .collect();
            (s, s, argb)
        })
        .collect()
}

/// File icons by name, drawn once per size and theme.
pub struct Icons {
    palette: niruicons::Palette,
    cache: HashMap<(&'static str, i32), Option<Rc<Image>>>,
}

impl Icons {
    pub fn new(p: &Palette) -> Self {
        Self {
            palette: niruicons::Palette {
                background: p.background,
                foreground: p.foreground,
                accent: p.mark(),
            },
            cache: HashMap::new(),
        }
    }

    pub fn get(&mut self, name: &str, folder: bool, size: i32) -> Option<Rc<Image>> {
        let kind = if folder { "folder" } else { kind_of(name) };
        if let Some(i) = self.cache.get(&(kind, size)) {
            return i.clone();
        }
        let img = niruicons::render(kind, size, &self.palette).map(|i| {
            Rc::new(Image {
                w: i.w,
                h: i.h,
                data: i.data,
            })
        });
        self.cache.insert((kind, size), img.clone());
        img
    }
}

/// The icon for a file name, by its extension.
pub fn kind_of(name: &str) -> &'static str {
    let ext = name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "heic" | "bmp" | "tif" | "tiff" | "svg"
        | "avif" | "raw" | "cr2" | "nef" | "dng" => "image-x-generic",
        "mp4" | "mkv" | "mov" | "webm" | "avi" | "m4v" => "video-x-generic",
        "mp3" | "flac" | "ogg" | "opus" | "wav" | "m4a" | "aac" => "audio-x-generic",
        "pdf" => "application-pdf",
        "zip" | "tar" | "gz" | "tgz" | "xz" | "zst" | "7z" | "rar" | "bz2" => "package-x-generic",
        "doc" | "docx" | "odt" | "rtf" => "x-office-document",
        "xls" | "xlsx" | "ods" | "csv" => "x-office-spreadsheet",
        "ppt" | "pptx" | "odp" => "x-office-presentation",
        "md" | "markdown" => "text-markdown",
        "html" | "htm" => "text-html",
        "rs" | "py" | "sh" | "c" | "h" | "cpp" | "js" | "ts" | "json" | "toml" | "yaml" | "yml"
        | "go" | "lua" | "css" | "xml" | "php" | "java" => "text-x-script",
        "iso" | "img" | "qcow2" => "application-x-cd-image",
        "ttf" | "otf" | "woff" | "woff2" => "font-x-generic",
        "txt" | "log" | "ini" | "conf" | "cfg" => "text-x-generic",
        "sqlite" | "db" | "sql" => "application-x-sqlite3",
        "ics" => "x-office-calendar",
        _ => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_sign_sits_inside_the_cloud() {
        for s in [Sign::Done, Sign::Busy, Sign::Paused, Sign::Error, Sign::Off] {
            let g = cells(s);
            let body = cells(Sign::Done);
            for y in 0..16 {
                for x in 0..16 {
                    if g[y][x] == 3 {
                        assert!(body[y][x] != 0, "{s:?} at {x},{y} is outside");
                    }
                }
            }
        }
        // Signed out: a dash across the middle of a full cloud.
        assert_eq!(cells(Sign::Off)[8][8], 3);
        assert_eq!(cells(Sign::Off)[11][8], 1);
        assert_eq!(kind_of("A.JPG"), "image-x-generic");
        assert_eq!(kind_of("noext"), "unknown");
    }
}
