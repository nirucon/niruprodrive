//! Drawing and input: the window's three tabs, the sign-in screen, the
//! tray icon and the pop-up menus, all in the colours of the NIRU theme.
use super::*;

const ESC: u32 = 0xff1b;
const ENTER: u32 = 0xff0d;
const KP_ENTER: u32 = 0xff8d;
const BACKSPACE: u32 = 0xff08;
const UP: u32 = 0xff52;
const DOWN: u32 = 0xff54;
const PGUP: u32 = 0xff55;
const PGDN: u32 = 0xff56;
const HOME: u32 = 0xff50;
const END: u32 = 0xff57;
const DELETE: u32 = 0xffff;
const F5: u32 = 0xffc2;
const TAB: u32 = 0xff09;

struct Layout {
    header: Rect,
    tabs: Rect,
    body: Rect,
    footer: Rect,
}

impl App {
    fn layout(&self) -> Layout {
        let (w, h) = (self.cv.w, self.cv.h);
        let pad = self.m.pad;
        let px = self.logo_px();
        let hh = 7 * px + 2 * pad + px;
        let th = self.m.row_h;
        let fh = self.m.small.height + pad + pad / 2;
        Layout {
            header: Rect::new(0, 0, w, hh),
            tabs: Rect::new(0, hh, w, th),
            body: Rect::new(0, hh + th, w, (h - hh - th - fh).max(0)),
            footer: Rect::new(0, h - fh, w, fh),
        }
    }

    fn logo_px(&self) -> i32 {
        ((3.0 * self.m.s).round() as i32).max(2)
    }

    // ------------------------------------------------------------ drawing

    pub(super) fn draw(&mut self) {
        self.hits.clear();
        let p = self.p;
        let l = self.layout();
        let (w, h) = (self.cv.w, self.cv.h);
        self.cv.reset_clip();
        self.cv.fill(Rect::new(0, 0, w, h), p.background);
        self.draw_header(l.header);
        self.draw_tabs(l.tabs);
        self.cv.set_clip(l.body);
        let blocked = !matches!(self.signed, Signed::In) && self.tab != Tab::Backup;
        if blocked {
            self.draw_signin(l.body);
        } else {
            match self.tab {
                Tab::Drive => self.draw_drive(l.body),
                Tab::Transfers => self.draw_transfers(l.body),
                Tab::Backup => self.draw_backup(l.body),
            }
        }
        self.cv.reset_clip();
        self.draw_footer(l.footer);
        if self.drop_hover {
            self.draw_drop(l.body);
        }
        if self.add_edit.is_some() {
            self.draw_add(l.body);
        }
        if self.confirm.is_some() {
            self.draw_confirm(l.body);
        }
    }

    fn draw_header(&mut self, r: Rect) {
        let p = self.p;
        let pad = self.m.pad;
        self.cv.fill(r, p.surface);
        self.cv
            .fill(Rect::new(r.x, r.y + r.h - 1, r.w, 1), p.border);
        let px = self.logo_px();
        let lw = logo::word_width("NIRUPRODRIVE", px);
        let (lx, ly) = (pad, r.y + (r.h - 7 * px) / 2);
        let cloud = 9 * px;
        let sign = self.sign();
        icons::draw_cloud(
            &mut self.cv,
            Rect::new(lx, r.y + (r.h - cloud) / 2, cloud, cloud),
            sign,
            &p,
            p.surface,
        );
        let tx = lx + cloud + pad;
        logo::word(
            &mut self.cv,
            tx,
            ly,
            "NIRUPRODRIVE",
            px,
            p.mark(),
            Some(blend(p.surface, p.mark(), 0.3)),
        );
        // Scanlines over the letters: the suite's retro touch.
        let mut sy = ly;
        while sy < ly + 7 * px {
            self.cv.tint(Rect::new(tx, sy, lw + px, 1), p.surface, 60);
            sy += 2;
        }
        let small = self.m.small;
        let status = self.status_line();
        let sw = self.text.width(&status, small);
        let max = r.w - (tx + lw + 3 * pad);
        let status = if sw > max {
            self.text.ellipsize(&status, small, max.max(0))
        } else {
            status
        };
        let sw = self.text.width(&status, small);
        self.text.draw(
            &mut self.cv,
            r.w - sw - pad,
            r.y + (r.h - small.height) / 2,
            &status,
            small,
            p.muted,
        );
    }

    fn status_line(&self) -> String {
        match &self.signed {
            Signed::NoCli => "Proton Drive CLI missing".into(),
            Signed::Unknown => "checking…".into(),
            Signed::Out => "signed out".into(),
            Signed::Failed(_) => "cannot reach Proton Drive".into(),
            Signed::In => {
                let mut s = self.cfg.host.clone();
                if let Some(u) = self.usage.filter(|u| *u > 0) {
                    s.push_str(&format!(" · {} backed up", human(u)));
                }
                if self.paused {
                    s.push_str(" · paused");
                }
                s
            }
        }
    }

    fn draw_tabs(&mut self, r: Rect) {
        let p = self.p;
        let (pad, font) = (self.m.pad, self.m.font);
        let u = (2.0 * self.m.s).max(2.0) as i32;
        let mut x = pad;
        for (i, (t, label)) in [
            (Tab::Drive, "DRIVE"),
            (Tab::Transfers, "TRANSFERS"),
            (Tab::Backup, "BACKUP"),
        ]
        .into_iter()
        .enumerate()
        {
            let num = format!("{} ", i + 1);
            let nw = self.text.width(&num, font);
            let lw = self.text.width(label, font);
            let tr = Rect::new(x, r.y, nw + lw + 2 * pad, r.h);
            let on = self.tab == t;
            self.text.draw(
                &mut self.cv,
                x + pad,
                r.y + (r.h - font.height) / 2,
                &num,
                font,
                if on {
                    p.mark()
                } else {
                    blend(p.muted, p.background, 0.4)
                },
            );
            self.text.draw(
                &mut self.cv,
                x + pad + nw,
                r.y + (r.h - font.height) / 2,
                label,
                font,
                if on { p.foreground } else { p.muted },
            );
            if on {
                self.cv.fill(
                    Rect::new(tr.x + pad, r.y + r.h - u - 1, nw + lw, u),
                    p.mark(),
                );
            }
            self.hits.push((tr, Hit::Act(Act::Tab(t))));
            x += tr.w;
        }
        // What is being sent, at the right.
        let busy = self.transfers.iter().find(|t| t.state == TState::Running);
        let waiting = self
            .transfers
            .iter()
            .filter(|t| t.state == TState::Queued)
            .count();
        let note = match busy {
            Some(t) if t.total > 0 => format!(
                "▲ {}% {}{}",
                (t.done * 100 / t.total).min(100),
                t.title,
                if waiting > 0 {
                    format!(" · {waiting} waiting")
                } else {
                    String::new()
                }
            ),
            Some(t) => format!("▲ {}", t.title),
            None => String::new(),
        };
        if !note.is_empty() {
            let small = self.m.small;
            let note = self.text.ellipsize(&note, small, r.w - x - 2 * pad);
            let nw = self.text.width(&note, small);
            self.text.draw(
                &mut self.cv,
                r.w - nw - pad,
                r.y + (r.h - small.height) / 2,
                &note,
                small,
                p.mark(),
            );
        }
        self.cv
            .fill(Rect::new(r.x, r.y + r.h - 1, r.w, 1), p.border);
    }

    fn big_word(&mut self, area: Rect, y: i32, word: &str, color: u32) {
        let px = ((area.w / 80).clamp(3, 7) as f32 * self.m.s.min(1.5)) as i32;
        let w = logo::word_width(word, px);
        let p = self.p;
        logo::word(
            &mut self.cv,
            area.x + (area.w - w) / 2,
            y,
            word,
            px,
            color,
            Some(blend(p.background, color, 0.25)),
        );
    }

    /// A paragraph, wrapped and centred. Returns the y below it.
    fn paragraph(&mut self, area: Rect, mut y: i32, text: &str, color: u32) -> i32 {
        let small = self.m.small;
        let max = (area.w - 6 * self.m.pad).min((560.0 * self.m.s) as i32);
        for line in self.text.wrap(text, small, max, 8) {
            let w = self.text.width(&line, small);
            self.text.draw(
                &mut self.cv,
                area.x + (area.w - w) / 2,
                y,
                &line,
                small,
                color,
            );
            y += small.height + 2;
        }
        y
    }

    /// A button: `[ KEY  label ]` in a pixel frame. Returns its width.
    fn button(&mut self, x: i32, y: i32, key: &str, label: &str, act: Act, strong: bool) -> i32 {
        let p = self.p;
        let (pad, small) = (self.m.pad, self.m.small);
        let kw = self.text.width(key, small);
        let lw = self.text.width(label, small);
        let w = kw + lw + 3 * pad;
        let h = small.height + pad;
        let r = Rect::new(x, y, w, h);
        let ground = if strong {
            blend(p.background, p.mark(), 0.22)
        } else {
            p.surface
        };
        self.cv.fill(r, ground);
        self.cv.frame(r, if strong { p.mark() } else { p.border });
        // A pixel notch at the corners, like old dialog buttons.
        let u = (self.m.s).max(1.0) as i32;
        for (cx, cy) in [
            (r.x, r.y),
            (r.x + r.w - u, r.y),
            (r.x, r.y + r.h - u),
            (r.x + r.w - u, r.y + r.h - u),
        ] {
            self.cv.fill(Rect::new(cx, cy, u, u), p.background);
        }
        self.text
            .draw(&mut self.cv, x + pad, y + pad / 2, key, small, p.mark());
        self.text.draw(
            &mut self.cv,
            x + 2 * pad + kw,
            y + pad / 2,
            label,
            small,
            p.foreground,
        );
        self.hits.push((r, Hit::Act(act)));
        w
    }

    fn draw_signin(&mut self, area: Rect) {
        let p = self.p;
        let pad = self.m.pad;
        let mut y = area.y + area.h / 6;
        match self.signed.clone() {
            Signed::Unknown => {
                self.big_word(area, area.y + area.h / 2 - 20, "CHECKING", p.muted);
            }
            Signed::NoCli => {
                self.big_word(area, y, "CLI MISSING", p.error);
                y += 12 * pad;
                self.paragraph(
                    area,
                    y,
                    "niruprodrive works through Proton's own Proton Drive CLI, and it is not installed. Run the niruprodrive installer again (nirux extras install niruprodrive): it fetches the CLI from proton.me and checks it.",
                    p.foreground,
                );
            }
            Signed::Failed(e) => {
                self.big_word(area, y, "NO CONTACT", p.warning);
                y += 12 * pad;
                y = self.paragraph(area, y, &e, p.foreground);
                y += pad;
                y = self.paragraph(
                    area,
                    y,
                    "Check the network. If the keyring is the problem, credentials = \"pass\" in the settings uses pass instead.",
                    p.muted,
                );
                let w = self.text.width("R  Try again", self.m.small) + 3 * pad;
                self.button(
                    area.x + (area.w - w) / 2,
                    y + 2 * pad,
                    "R",
                    "Try again",
                    Act::Retry,
                    true,
                );
            }
            Signed::Out | Signed::In => {
                self.big_word(area, y, "SIGN IN", p.mark());
                y += 12 * pad;
                if self.logging_in {
                    y = self.paragraph(
                        area,
                        y,
                        "Sign in on Proton's page in your browser. This window carries on by itself when you are done.",
                        p.foreground,
                    );
                    if let Some(u) = self.login_url.clone() {
                        y += pad;
                        let small = self.m.small;
                        let shown = self.text.ellipsize(&u, small, area.w - 8 * pad);
                        let w = self.text.width(&shown, small);
                        self.text.draw(
                            &mut self.cv,
                            area.x + (area.w - w) / 2,
                            y,
                            &shown,
                            small,
                            p.muted,
                        );
                        y += small.height + 2 * pad;
                        let w1 = self.text.width("C  Copy the address", small) + 3 * pad;
                        let w2 = self.text.width("O  Open it again", small) + 3 * pad;
                        let x0 = area.x + (area.w - w1 - w2 - pad) / 2;
                        self.button(x0, y, "C", "Copy the address", Act::CopyLoginUrl, false);
                        self.button(
                            x0 + w1 + pad,
                            y,
                            "O",
                            "Open it again",
                            Act::OpenLoginUrl,
                            false,
                        );
                    }
                } else {
                    y = self.paragraph(
                        area,
                        y,
                        "niruprodrive backs up this computer's folders to Proton Drive and sends, fetches and shares files. Sign-in happens on Proton's own page in your browser, with two-factor if you use it: niruprodrive never sees your password.",
                        p.foreground,
                    );
                    y += pad;
                    y = self.paragraph(
                        area,
                        y,
                        "niruprodrive is a third-party application. It is not made or supported by Proton; it works through Proton's Proton Drive CLI.",
                        p.muted,
                    );
                    let label = "Sign in with Proton";
                    let w = self.text.width(&format!("Enter  {label}"), self.m.small) + 3 * pad;
                    self.button(
                        area.x + (area.w - w) / 2,
                        y + 2 * pad,
                        "Enter",
                        label,
                        Act::SignIn,
                        true,
                    );
                }
            }
        }
    }

    fn draw_drive(&mut self, area: Rect) {
        let p = self.p;
        let (pad, font, small, row_h) = (self.m.pad, self.m.font, self.m.small, self.m.row_h);
        // The path, each part clickable, and the shortcuts on the right.
        let bar = Rect::new(area.x, area.y, area.w, row_h);
        let mut x = pad;
        let mut acc = String::new();
        let parts: Vec<String> = split_path(&self.path);
        for (i, part) in parts.iter().enumerate() {
            if i == 0 {
                acc = "/my-files".into();
            } else {
                acc.push('/');
                acc.push_str(&cli::escape(part));
            }
            let label = if i == 0 {
                "My files".to_string()
            } else {
                part.clone()
            };
            if i > 0 {
                let sep = " / ";
                self.text.draw(
                    &mut self.cv,
                    x,
                    bar.y + (row_h - font.height) / 2,
                    sep,
                    font,
                    p.border,
                );
                x += self.text.width(sep, font);
            }
            let w = self.text.width(&label, font);
            let last = i == parts.len() - 1;
            self.text.draw(
                &mut self.cv,
                x,
                bar.y + (row_h - font.height) / 2,
                &label,
                font,
                if last { p.foreground } else { p.muted },
            );
            self.hits.push((
                Rect::new(x, bar.y, w, row_h),
                Hit::Act(Act::Go(acc.clone())),
            ));
            x += w;
        }
        let mut rx = area.w - pad;
        for (label, path) in [
            ("UPLOADS", self.cfg.remote_uploads()),
            ("THIS COMPUTER", self.cfg.remote_host()),
            ("NIRUX", self.cfg.remote_base()),
        ] {
            let w = self.text.width(label, small) + pad;
            rx -= w;
            if rx < x + 2 * pad {
                break;
            }
            let here = self.path == path;
            self.text.draw(
                &mut self.cv,
                rx,
                bar.y + (row_h - small.height) / 2,
                label,
                small,
                if here { p.mark() } else { p.muted },
            );
            self.hits
                .push((Rect::new(rx, bar.y, w, row_h), Hit::Act(Act::Go(path))));
            rx -= pad;
        }
        self.cv
            .fill(Rect::new(area.x, bar.y + row_h - 1, area.w, 1), p.border);
        let list = Rect::new(area.x, area.y + row_h, area.w, area.h - row_h);
        if self.nodes.is_empty() {
            let (word, col) = if self.listing.is_some() {
                ("LOADING", p.muted)
            } else if self.list_error.is_some() {
                ("NOT THERE", p.error)
            } else {
                ("EMPTY", p.muted)
            };
            self.big_word(list, list.y + list.h / 3, word, col);
            let msg = match &self.list_error {
                Some(e) => e.clone(),
                None if self.listing.is_none() && cli::within(&self.path, &self.cfg.remote_base()) => {
                    "Backups and files sent from NIRUFM land here. Drop files on this window to send them.".into()
                }
                None => String::new(),
            };
            if !msg.is_empty() {
                self.paragraph(list, list.y + list.h / 3 + 12 * pad, &msg, p.muted);
            }
            return;
        }
        // Keep the selection in view.
        let rows = (list.h / row_h).max(1);
        let sel = self.sel as i32;
        if sel * row_h < self.scroll {
            self.scroll = sel * row_h;
        } else if (sel + 1) * row_h > self.scroll + rows * row_h {
            self.scroll = (sel + 1 - rows) * row_h;
        }
        let max = (self.nodes.len() as i32 * row_h - list.h).max(0);
        self.scroll = self.scroll.clamp(0, max);
        let isz = 16 * ((self.m.s).round() as i32).max(1);
        let size_w = self.text.width("000.0 MB", small) + 2 * pad;
        let date_w = self.text.width("2026-11-24 12:00", small) + 2 * pad;
        let link_w = self.text.width("LINK", small) + 2 * pad;
        let name_w = list.w - size_w - date_w - link_w - isz - 4 * pad;
        let first = (self.scroll / row_h) as usize;
        for i in first..self.nodes.len() {
            let y = list.y + i as i32 * row_h - self.scroll;
            if y > list.y + list.h {
                break;
            }
            let n = self.nodes[i].clone();
            let r = Rect::new(list.x, y, list.w, row_h);
            if i == self.sel {
                self.cv.fill(r, p.selected);
                self.cv.fill(
                    Rect::new(r.x, r.y, (2.0 * self.m.s).max(2.0) as i32, r.h),
                    p.mark(),
                );
            } else if i % 2 == 1 {
                self.cv.fill(r, blend(p.background, p.surface, 0.5));
            }
            if let Some(img) = self.icons.get(&n.name, n.folder, isz) {
                self.cv.image(pad * 2, y + (row_h - img.h) / 2, &img, 255);
            }
            let tx = 3 * pad + isz;
            let name = self.text.ellipsize(&n.name, font, name_w);
            self.text.draw(
                &mut self.cv,
                tx,
                y + (row_h - font.height) / 2,
                &name,
                font,
                p.foreground,
            );
            let ty = y + (row_h - small.height) / 2;
            if !n.folder {
                let s = human(n.size);
                let sw = self.text.width(&s, small);
                self.text.draw(
                    &mut self.cv,
                    tx + name_w + size_w - sw,
                    ty,
                    &s,
                    small,
                    p.muted,
                );
            }
            self.text.draw(
                &mut self.cv,
                tx + name_w + size_w + 2 * pad,
                ty,
                &n.modified,
                small,
                p.muted,
            );
            if n.link {
                let lx = list.w - link_w;
                let lr = Rect::new(lx, ty - 1, link_w - pad, small.height + 2);
                self.cv.frame(lr, p.mark());
                self.text
                    .draw(&mut self.cv, lx + pad / 2, ty, "LINK", small, p.mark());
            }
            self.hits.push((r.intersect(&list), Hit::Row(i)));
        }
    }

    fn draw_transfers(&mut self, area: Rect) {
        let p = self.p;
        let (pad, font, small) = (self.m.pad, self.m.font, self.m.small);
        if self.transfers.is_empty() {
            self.big_word(area, area.y + area.h / 3, "NOTHING YET", p.muted);
            self.paragraph(
                area,
                area.y + area.h / 3 + 12 * pad,
                "Backups, files sent from NIRUFM, downloads and links show here.",
                p.muted,
            );
            return;
        }
        let rh = font.height + small.height + 2 * pad;
        let rows = (area.h / rh).max(1) as usize;
        let first = self.tsel.saturating_sub(rows - 1);
        for (k, i) in (first..self.transfers.len()).enumerate() {
            let y = area.y + k as i32 * rh;
            if y > area.y + area.h {
                break;
            }
            let t = &self.transfers[i];
            let (tag, col) = match t.state {
                TState::Queued => ("WAIT", p.muted),
                TState::Running => ("BUSY", p.mark()),
                TState::Done => ("DONE", p.success),
                TState::Failed => ("FAIL", p.error),
            };
            let title = t.title.clone();
            let detail = match t.state {
                TState::Running if t.total > 0 || t.of > 0 => format!(
                    "{} of {} · {} of {} · {}",
                    human(t.done),
                    human(t.total),
                    t.files,
                    plural(t.of, "file"),
                    t.now
                ),
                TState::Running => "working…".to_string(),
                TState::Queued => "waiting for the one before it".to_string(),
                _ => match &t.url {
                    Some(u) => format!("{} · {u}", t.msg),
                    None => t.msg.clone(),
                },
            };
            let (state, done, total, when) = (t.state, t.done, t.total, t.when);
            let r = Rect::new(area.x, y, area.w, rh);
            if i == self.tsel {
                self.cv.fill(r, p.selected);
            }
            let tagw = self.text.width("BUSY", small) + pad;
            let tr = Rect::new(pad, y + pad / 2, tagw, small.height + 2);
            self.cv.frame(tr, col);
            self.text.draw(
                &mut self.cv,
                pad + pad / 2,
                y + pad / 2 + 1,
                tag,
                small,
                col,
            );
            let tx = 2 * pad + tagw;
            let time = local_time(when)[11..].to_string();
            let tw = self.text.width(&time, small);
            let title = self
                .text
                .ellipsize(&title, font, area.w - tx - tw - 3 * pad);
            self.text
                .draw(&mut self.cv, tx, y + pad / 2, &title, font, p.foreground);
            self.text.draw(
                &mut self.cv,
                area.w - tw - pad,
                y + pad / 2,
                &time,
                small,
                p.muted,
            );
            let detail = self.text.ellipsize(&detail, small, area.w - tx - pad);
            let dy = y + pad / 2 + font.height + 2;
            self.text.draw(
                &mut self.cv,
                tx,
                dy,
                &detail,
                small,
                if state == TState::Failed {
                    p.error
                } else {
                    p.muted
                },
            );
            if state == TState::Running && total > 0 {
                // A pixel progress bar along the bottom of the row.
                let u = (2.0 * self.m.s).max(2.0) as i32;
                let bw = area.w - tx - pad;
                let by = y + rh - u - 2;
                self.cv.fill(Rect::new(tx, by, bw, u), p.border);
                let fill = (bw as u64 * done.min(total) / total.max(1)) as i32;
                let mut bx = tx;
                while bx < tx + fill {
                    self.cv
                        .fill(Rect::new(bx, by, (2 * u).min(tx + fill - bx), u), p.mark());
                    bx += 3 * u;
                }
            }
            self.hits.push((r.intersect(&area), Hit::TRow(i)));
        }
    }

    fn draw_backup(&mut self, area: Rect) {
        let p = self.p;
        let (pad, font, small, row_h) = (self.m.pad, self.m.font, self.m.small, self.m.row_h);
        let mut y = area.y + pad;
        let head = |me: &mut Self, y: i32, s: &str| {
            me.text.draw(&mut me.cv, 2 * pad, y, s, small, p.mark());
        };
        head(self, y, "FOLDERS BACKED UP FROM THIS COMPUTER");
        y += small.height + pad / 2;
        if self.cfg.folders.is_empty() {
            self.text.draw(
                &mut self.cv,
                2 * pad,
                y + (row_h - small.height) / 2,
                "None yet: drop folders here from NIRUFM, or press A and type one.",
                small,
                p.muted,
            );
            y += row_h;
        }
        let folders = self.cfg.folders.clone();
        let st = self
            .shared
            .state
            .lock()
            .map(|s| s.clone())
            .unwrap_or_default();
        for (i, f) in folders.iter().enumerate() {
            let r = Rect::new(area.x, y, area.w, row_h);
            if i == self.bsel {
                self.cv.fill(r, p.selected);
                self.cv.fill(
                    Rect::new(r.x, r.y, (2.0 * self.m.s).max(2.0) as i32, r.h),
                    p.mark(),
                );
            }
            let isz = 16 * ((self.m.s).round() as i32).max(1);
            if let Some(img) = self.icons.get("", true, isz) {
                self.cv.image(2 * pad, y + (row_h - img.h) / 2, &img, 255);
            }
            let name = tilde(f);
            self.text.draw(
                &mut self.cv,
                3 * pad + isz,
                y + (row_h - font.height) / 2,
                &name,
                font,
                p.foreground,
            );
            let (note, col) = if !f.is_dir() {
                ("not found".to_string(), p.error)
            } else {
                let n = st.files.keys().filter(|k| k.starts_with(f)).count() as u64;
                (format!("{} sent", plural(n, "file")), p.muted)
            };
            let nw = self.text.width(&note, small);
            self.text.draw(
                &mut self.cv,
                area.w - nw - 2 * pad,
                y + (row_h - small.height) / 2,
                &note,
                small,
                col,
            );
            self.hits.push((r, Hit::BRow(i)));
            y += row_h;
        }
        y += pad / 2;
        let mut bx = 2 * pad;
        bx += self.button(bx, y, "A", "Add a folder", Act::AddFolder, false) + pad;
        if !folders.is_empty() {
            self.button(bx, y, "Del", "Stop backing up", Act::RemoveFolder, false);
        }
        y += small.height + 3 * pad;
        head(self, y, "SETTINGS FOR THIS COMPUTER");
        y += small.height + pad / 2;
        let next = match (self.cfg.every.seconds(), self.paused, self.due()) {
            (None, _, _) => "only when you ask".to_string(),
            (_, true, _) => "paused (P resumes)".to_string(),
            (_, _, Some(d)) if d <= now() => "now".to_string(),
            (_, _, Some(d)) => local_time(d),
            _ if self.backup_id.is_some() => "running now".into(),
            _ if self.signed != Signed::In => "after sign-in".into(),
            _ => "when a folder is added".into(),
        };
        let last = if st.last_run == 0 {
            "never".to_string()
        } else if st.last_error.is_empty() {
            format!(
                "{} · {} · {}",
                local_time(st.last_run),
                plural(st.last_files, "file"),
                human(st.last_bytes)
            )
        } else {
            format!("{} · {}", local_time(st.last_run), st.last_error)
        };
        type Row<'a> = (&'a str, String, Option<(&'a str, Act)>, u32);
        let rows: Vec<Row> = vec![
            (
                "Schedule",
                self.cfg.every.name().to_string(),
                Some(("S", Act::CycleEvery)),
                p.foreground,
            ),
            (
                "On battery",
                if self.cfg.on_battery {
                    "back up"
                } else {
                    "wait for power"
                }
                .to_string(),
                Some(("O", Act::ToggleBattery)),
                p.foreground,
            ),
            ("Next backup", next, None, p.foreground),
            (
                "Last backup",
                last,
                None,
                if st.last_error.is_empty() {
                    p.foreground
                } else {
                    p.error
                },
            ),
            (
                "In the Drive",
                shown(&self.cfg.remote_host()),
                None,
                p.foreground,
            ),
            (
                "Left out",
                format!(
                    "{} · files over {} MB",
                    self.cfg.exclude.join(" "),
                    self.cfg.max_file_mb
                ),
                Some(("E", Act::EditConfig)),
                p.muted,
            ),
        ];
        let lw = self.text.width("In the Drive  ", small) + 2 * pad;
        for (label, value, key, col) in rows {
            let r = Rect::new(area.x, y, area.w, small.height + pad);
            self.text
                .draw(&mut self.cv, 2 * pad, y + pad / 2, label, small, p.muted);
            let mut vx = 2 * pad + lw;
            if let Some((k, act)) = key {
                let kr = Rect::new(
                    vx,
                    y + pad / 2 - 1,
                    self.text.width(k, small) + pad,
                    small.height + 2,
                );
                self.cv.frame(kr, p.border);
                self.text
                    .draw(&mut self.cv, vx + pad / 2, y + pad / 2, k, small, p.mark());
                vx += kr.w + pad;
                self.hits.push((r, Hit::Act(act)));
            }
            let value = self.text.ellipsize(&value, small, area.w - vx - 2 * pad);
            self.text
                .draw(&mut self.cv, vx, y + pad / 2, &value, small, col);
            y += small.height + pad;
        }
        y += pad;
        let mut bx = 2 * pad;
        if self.backup_id.is_some() {
            bx += self.button(bx, y, "Esc", "Stop the backup", Act::Cancel, true) + pad;
        } else {
            bx += self.button(bx, y, "B", "Back up now", Act::BackupNow, true) + pad;
        }
        bx += self.button(
            bx,
            y,
            "P",
            if self.paused { "Resume" } else { "Pause" },
            Act::Pause,
            false,
        ) + pad;
        if self.signed == Signed::In {
            self.button(bx, y, "Ctrl+O", "Sign out", Act::SignOut, false);
        } else {
            self.button(bx, y, "Enter", "Sign in", Act::SignIn, false);
        }
        y += small.height + 3 * pad;
        let note = "Nothing is ever removed in the Drive: deleted files stay there, changed files keep their old versions. Restore from the Drive tab (This computer, then D).";
        for line in self.text.wrap(note, small, area.w - 4 * pad, 3) {
            if y + small.height > area.y + area.h {
                break;
            }
            self.text.draw(
                &mut self.cv,
                2 * pad,
                y,
                &line,
                small,
                blend(p.muted, p.background, 0.3),
            );
            y += small.height + 2;
        }
    }

    fn draw_footer(&mut self, r: Rect) {
        let p = self.p;
        let (pad, small) = (self.m.pad, self.m.small);
        self.cv.fill(r, p.surface);
        self.cv.fill(Rect::new(r.x, r.y, r.w, 1), p.border);
        let ty = r.y + (r.h - small.height) / 2;
        if let Some((m, _, err)) = self.message.clone() {
            let m = self.text.ellipsize(&m, small, r.w - 2 * pad);
            self.text.draw(
                &mut self.cv,
                pad,
                ty,
                &m,
                small,
                if err { p.error } else { p.mark() },
            );
            return;
        }
        let hints = match (self.tab, &self.signed) {
            (Tab::Drive, Signed::In) => "Enter open · Backspace up · D download · C copy link · Shift+C link for 7 days · X remove link · F5 refresh · drop files to send them here",
            (Tab::Transfers, _) => "Esc stops what runs · C copies a link · Delete clears the finished",
            (Tab::Backup, _) => "A add · Delete stop · B back up now · P pause · S schedule · O battery · E settings file",
            _ => "1 2 3 tabs · Ctrl+Q quit · Esc closes the window",
        };
        let hints = self.text.ellipsize(hints, small, r.w - 2 * pad);
        self.text
            .draw(&mut self.cv, pad, ty, &hints, small, p.muted);
    }

    fn draw_drop(&mut self, area: Rect) {
        let p = self.p;
        let inner = area.inset(self.m.pad);
        self.cv.tint(inner, p.mark(), 30);
        self.cv
            .dashed(inner, p.mark(), (4.0 * self.m.s).max(3.0) as i32);
        let word = if self.tab == Tab::Backup {
            "BACK UP"
        } else {
            "SEND HERE"
        };
        self.big_word(area, area.y + area.h / 2 - 20, word, p.mark());
    }

    fn dialog(&mut self, area: Rect, lines: usize) -> Rect {
        let p = self.p;
        let (pad, small) = (self.m.pad, self.m.small);
        let w = (area.w - 8 * pad).min((520.0 * self.m.s) as i32);
        let h = (lines as i32) * (small.height + 2) + 6 * pad + small.height;
        let r = Rect::new(area.x + (area.w - w) / 2, area.y + (area.h - h) / 2, w, h);
        self.cv.tint(area, p.background, 120);
        // A hard pixel shadow and a double frame.
        let u = (2.0 * self.m.s).max(2.0) as i32;
        self.cv.fill(
            Rect::new(r.x + u, r.y + u, r.w, r.h),
            blend(p.background, 0x000000, 0.5),
        );
        self.cv.fill(r, p.surface);
        self.cv.frame(r, p.mark());
        self.cv.frame(r.inset(u), p.border);
        r
    }

    fn draw_confirm(&mut self, area: Rect) {
        let Some(text) = self.confirm.as_ref().map(|c| c.text.clone()) else {
            return;
        };
        let p = self.p;
        let (pad, small) = (self.m.pad, self.m.small);
        let lines = self
            .text
            .wrap(&text, small, (500.0 * self.m.s) as i32 - 6 * pad, 6);
        let r = self.dialog(area, lines.len());
        let mut y = r.y + 2 * pad;
        for l in &lines {
            self.text
                .draw(&mut self.cv, r.x + 2 * pad, y, l, small, p.foreground);
            y += small.height + 2;
        }
        y += pad;
        let w = self.button(r.x + 2 * pad, y, "Y", "Yes", Act::Yes, true);
        self.button(r.x + 3 * pad + w, y, "N", "No", Act::No, false);
    }

    fn draw_add(&mut self, area: Rect) {
        let Some(s) = self.add_edit.clone() else {
            return;
        };
        let p = self.p;
        let (pad, small, font) = (self.m.pad, self.m.small, self.m.font);
        let r = self.dialog(area, 2);
        self.text.draw(
            &mut self.cv,
            r.x + 2 * pad,
            r.y + 2 * pad,
            "A folder to back up (Tab completes, Enter adds, Esc stops):",
            small,
            p.muted,
        );
        let fr = Rect::new(
            r.x + 2 * pad,
            r.y + 3 * pad + small.height,
            r.w - 4 * pad,
            font.height + pad,
        );
        self.cv.fill(fr, p.background);
        self.cv.frame(fr, p.border);
        let shown_s = self.text.ellipsize(&s, font, fr.w - 2 * pad);
        let tw = self.text.draw(
            &mut self.cv,
            fr.x + pad,
            fr.y + pad / 2,
            &shown_s,
            font,
            p.foreground,
        );
        // A block cursor, as in a terminal.
        let cw = self.text.width("M", font);
        self.cv.fill(
            Rect::new(fr.x + pad + tw, fr.y + pad / 2, cw / 2 + 1, font.height),
            p.mark(),
        );
        let ok = Path::new(&expand_tilde(&s)).is_dir();
        let note = if ok { "a folder" } else { "no such folder yet" };
        self.text.draw(
            &mut self.cv,
            fr.x,
            fr.y + fr.h + pad / 2,
            note,
            small,
            if ok { p.success } else { p.muted },
        );
    }

    // ------------------------------------------------------------ tray

    pub(super) fn sign(&self) -> Sign {
        match &self.signed {
            Signed::NoCli | Signed::Out | Signed::Unknown => return Sign::Off,
            Signed::Failed(_) => return Sign::Error,
            Signed::In => {}
        }
        if self
            .transfers
            .iter()
            .any(|t| matches!(t.state, TState::Running | TState::Queued))
        {
            return Sign::Busy;
        }
        if self.paused {
            return Sign::Paused;
        }
        let err = self
            .shared
            .state
            .lock()
            .map(|s| !s.last_error.is_empty())
            .unwrap_or(false);
        if err {
            Sign::Error
        } else {
            Sign::Done
        }
    }

    pub(super) fn draw_tray(&mut self) {
        self.tray_dirty = false;
        if !self.x.docked {
            return;
        }
        let sign = self.sign();
        let (w, h) = self.x.tray_size();
        if self.tray_sign == Some(sign) && self.cv_tray_size == (w, h) {
            return;
        }
        self.tray_sign = Some(sign);
        self.cv_tray_size = (w, h);
        let mut cv = Canvas::new(w, h);
        let p = self.p;
        cv.fill(Rect::new(0, 0, w, h), p.background);
        let side = w.min(h);
        let u = (side / 16).max(1);
        let size = if side >= 16 { 16 * u } else { side };
        icons::draw_cloud(
            &mut cv,
            Rect::new((w - size) / 2, (h - size) / 2, size, size),
            sign,
            &p,
            p.background,
        );
        self.x.present(self.x.tray, &cv);
        // The bar shows the window name as the icon's tooltip, if it can.
        let tip = format!("niruprodrive: {}", self.status_line());
        use x11rb::wrapper::ConnectionExt as _;
        let _ = self.x.c.change_property8(
            x11rb::protocol::xproto::PropMode::REPLACE,
            self.x.tray,
            self.x.atoms.net_wm_name,
            self.x.atoms.utf8,
            tip.as_bytes(),
        );
    }

    // ------------------------------------------------------------ menus

    fn open_menu(&mut self, rx: i32, ry: i32, items: Vec<(String, Act)>) {
        let (pad, small) = (self.m.pad, self.m.small);
        let w = items
            .iter()
            .map(|(s, _)| self.text.width(s, small))
            .max()
            .unwrap_or(100)
            + 5 * pad;
        let ih = small.height + pad;
        let h = items.len() as i32 * ih + pad;
        let (sw, sh) = self.x.screen_size();
        let x = (rx - w / 2).clamp(0, (sw - w).max(0));
        let y = if ry > sh / 2 { ry - h - 4 } else { ry + 4 };
        let y = y.clamp(0, (sh - h).max(0));
        self.menu = Some(Menu {
            items,
            hover: None,
            w,
            h,
        });
        self.x.show_menu(x, y, w, h);
        self.draw_menu();
    }

    pub(super) fn draw_menu(&mut self) {
        let Some(m) = &self.menu else { return };
        let (w, h) = (m.w, m.h);
        let items: Vec<String> = m.items.iter().map(|i| i.0.clone()).collect();
        let hover = m.hover;
        let p = self.p;
        let (pad, small) = (self.m.pad, self.m.small);
        let mut cv = Canvas::new(w, h);
        cv.fill(Rect::new(0, 0, w, h), p.surface);
        cv.frame(Rect::new(0, 0, w, h), p.mark());
        self.menu_hits.clear();
        let ih = small.height + pad;
        for (i, s) in items.iter().enumerate() {
            let r = Rect::new(1, pad / 2 + i as i32 * ih, w - 2, ih);
            if hover == Some(i) {
                cv.fill(r, p.selected);
                cv.fill(
                    Rect::new(r.x + pad / 2, r.y + 2, (2.0 * self.m.s) as i32, r.h - 4),
                    p.mark(),
                );
            }
            self.text
                .draw(&mut cv, 2 * pad, r.y + pad / 2, s, small, p.foreground);
            self.menu_hits.push((r, i));
        }
        self.x.present(self.x.menu, &cv);
    }

    pub(super) fn close_menu(&mut self) {
        if self.menu.is_some() {
            self.x.hide_menu();
        }
    }

    pub(super) fn menu_act(&mut self, i: usize) -> Option<Act> {
        let m = self.menu.take()?;
        m.items.get(i).map(|x| x.1.clone())
    }

    pub(super) fn menu_key(&mut self, sym: u32) {
        let Some(m) = self.menu.as_mut() else { return };
        let n = m.items.len();
        match sym {
            ESC => {
                self.close_menu();
                self.menu = None;
            }
            UP => {
                m.hover = Some(m.hover.map_or(n - 1, |h| (h + n - 1) % n));
                self.draw_menu();
            }
            DOWN => {
                m.hover = Some(m.hover.map_or(0, |h| (h + 1) % n));
                self.draw_menu();
            }
            ENTER | KP_ENTER => {
                let h = m.hover;
                self.close_menu();
                if let Some(a) = h.and_then(|i| self.menu_act(i)) {
                    self.act(a);
                }
                self.menu = None;
            }
            _ => {}
        }
    }

    pub(super) fn tray_menu(&mut self, rx: i32, ry: i32) {
        let mut items: Vec<(String, Act)> = vec![(
            if self.x.mapped {
                "Hide the window"
            } else {
                "Open niruprodrive"
            }
            .into(),
            Act::Show,
        )];
        if self.signed == Signed::In {
            if self.backup_id.is_some() {
                items.push(("Stop the backup".into(), Act::Cancel));
            } else {
                items.push(("Back up now".into(), Act::BackupNow));
            }
            items.push((
                if self.paused {
                    "Resume backups"
                } else {
                    "Pause backups"
                }
                .into(),
                Act::Pause,
            ));
        } else if self.runner.is_some() {
            items.push(("Sign in…".into(), Act::SignIn));
        }
        items.push(("Quit".into(), Act::Quit));
        self.open_menu(rx, ry, items);
    }

    fn row_menu(&mut self, rx: i32, ry: i32) {
        let Some(n) = self.nodes.get(self.sel).cloned() else {
            return;
        };
        let mut items: Vec<(String, Act)> = vec![(
            if n.folder { "Open" } else { "Download" }.into(),
            if n.folder {
                Act::Go(self.node_path(self.sel).unwrap_or_default())
            } else {
                Act::Download
            },
        )];
        if n.folder {
            items.push(("Download the folder".into(), Act::Download));
        }
        items.push(("Copy a link".into(), Act::Link(None)));
        items.push(("Copy a link for 7 days".into(), Act::Link(Some(7))));
        items.push(("Copy a link for 30 days".into(), Act::Link(Some(30))));
        if n.link {
            items.push(("Remove the link…".into(), Act::Unlink));
        }
        self.open_menu(rx, ry, items);
    }

    // ------------------------------------------------------------ input

    pub(super) fn click(&mut self, b: u8, x: i32, y: i32, rx: i32, ry: i32) {
        self.dirty = true;
        let hit = self
            .hits
            .iter()
            .rev()
            .find(|(r, _)| r.contains(x, y))
            .map(|(_, h)| h.clone());
        if self.confirm.is_some() || self.add_edit.is_some() {
            if let Some(Hit::Act(a @ (Act::Yes | Act::No))) = hit {
                self.act(a);
            }
            return;
        }
        let row_h = self.m.row_h;
        match (b, hit) {
            (4, _) => self.wheel(-1),
            (5, _) => self.wheel(1),
            (1, Some(Hit::Act(a))) => self.act(a),
            (1, Some(Hit::Row(i))) => {
                let double = self
                    .last_click
                    .is_some_and(|(t, j)| j == i && t.elapsed() < DOUBLE_CLICK);
                self.sel = i;
                if double {
                    self.last_click = None;
                    self.open(i);
                } else {
                    self.last_click = Some((Instant::now(), i));
                }
            }
            (3, Some(Hit::Row(i))) => {
                self.sel = i;
                self.draw();
                self.x.present(self.x.win, &self.cv);
                self.row_menu(rx, ry);
            }
            (1, Some(Hit::BRow(i))) => self.bsel = i,
            (1, Some(Hit::TRow(i))) => {
                self.tsel = i;
                let double = self
                    .last_click
                    .is_some_and(|(t, j)| j == i + 100_000 && t.elapsed() < DOUBLE_CLICK);
                if double {
                    if let Some(id) = self
                        .transfers
                        .get(i)
                        .filter(|t| t.url.is_some())
                        .map(|t| t.id)
                    {
                        self.act(Act::CopyUrl(id));
                    }
                }
                self.last_click = Some((Instant::now(), i + 100_000));
            }
            _ => {}
        }
        let _ = row_h;
    }

    fn wheel(&mut self, d: i32) {
        match self.tab {
            Tab::Drive => {
                self.scroll = (self.scroll + d * 3 * self.m.row_h).max(0);
                let rows = (self.layout().body.h / self.m.row_h - 1).max(1) as usize;
                let top = (self.scroll / self.m.row_h) as usize;
                self.sel = self
                    .sel
                    .clamp(top, top + rows - 1)
                    .min(self.nodes.len().saturating_sub(1));
            }
            Tab::Transfers => {
                self.tsel = (self.tsel as i32 + d)
                    .clamp(0, self.transfers.len().saturating_sub(1) as i32)
                    as usize;
            }
            Tab::Backup => {}
        }
    }

    pub(super) fn key(&mut self, code: u8, state: KeyButMask) {
        self.dirty = true;
        let sym = self.x.keysym(code, state);
        let ctrl = state.contains(KeyButMask::CONTROL);
        let shift = state.contains(KeyButMask::SHIFT);
        if self.confirm.is_some() {
            match sym {
                0x79 | 0x59 | ENTER | KP_ENTER => self.act(Act::Yes),
                0x6e | 0x4e | ESC => self.act(Act::No),
                _ => {}
            }
            return;
        }
        if let Some(s) = self.add_edit.as_mut() {
            match sym {
                ESC => self.add_edit = None,
                BACKSPACE => {
                    if ctrl {
                        let t = s.trim_end_matches('/');
                        let cut = t.rfind('/').map_or(0, |i| i + 1);
                        s.truncate(cut);
                    } else {
                        s.pop();
                    }
                }
                TAB => {
                    let done = complete(s);
                    *s = done;
                }
                ENTER | KP_ENTER => {
                    let p = PathBuf::from(expand_tilde(s));
                    if p.is_dir() {
                        self.add_edit = None;
                        self.take_files_backup(p);
                    } else {
                        self.flash("No such folder", true);
                    }
                }
                _ => {
                    if let Some(c) = char::from_u32(sym).filter(|c| !c.is_control() && sym < 0xff00)
                    {
                        if !ctrl {
                            s.push(c);
                        }
                    }
                }
            }
            return;
        }
        if ctrl {
            match sym {
                0x71 | 0x51 => self.act(Act::Quit),
                0x77 | 0x57 => self.x.hide(),
                0x6f | 0x4f => self.act(Act::SignOut),
                _ => {}
            }
            return;
        }
        match sym {
            0x31 => return self.act(Act::Tab(Tab::Drive)),
            0x32 => return self.act(Act::Tab(Tab::Transfers)),
            0x33 => return self.act(Act::Tab(Tab::Backup)),
            TAB => {
                let t = match (self.tab, shift) {
                    (Tab::Drive, false) | (Tab::Backup, true) => Tab::Transfers,
                    (Tab::Transfers, false) | (Tab::Drive, true) => Tab::Backup,
                    _ => Tab::Drive,
                };
                return self.act(Act::Tab(t));
            }
            _ => {}
        }
        let signed_in = self.signed == Signed::In;
        if !signed_in && self.tab != Tab::Backup {
            match sym {
                ENTER | KP_ENTER if !self.logging_in => self.act(Act::SignIn),
                0x63 if self.logging_in => self.act(Act::CopyLoginUrl),
                0x6f if self.logging_in => self.act(Act::OpenLoginUrl),
                0x72 => self.act(Act::Retry),
                ESC => self.x.hide(),
                _ => {}
            }
            return;
        }
        match self.tab {
            Tab::Drive => {
                let n = self.nodes.len();
                let page = (self.layout().body.h / self.m.row_h - 2).max(1) as usize;
                match sym {
                    UP => self.sel = self.sel.saturating_sub(1),
                    DOWN if n > 0 => self.sel = (self.sel + 1).min(n - 1),
                    PGUP => self.sel = self.sel.saturating_sub(page),
                    PGDN if n > 0 => self.sel = (self.sel + page).min(n - 1),
                    HOME => self.sel = 0,
                    END => self.sel = n.saturating_sub(1),
                    ENTER | KP_ENTER => self.open(self.sel),
                    BACKSPACE => self.up(),
                    F5 => self.act(Act::Refresh),
                    0x64 | 0x44 => self.act(Act::Download),
                    0x63 => self.act(Act::Link(None)),
                    0x43 => self.act(Act::Link(Some(7))),
                    0x78 | 0x58 => self.act(Act::Unlink),
                    ESC => self.x.hide(),
                    _ => {}
                }
            }
            Tab::Transfers => {
                let n = self.transfers.len();
                match sym {
                    UP => self.tsel = self.tsel.saturating_sub(1),
                    DOWN if n > 0 => self.tsel = (self.tsel + 1).min(n - 1),
                    ESC if self.transfers.iter().any(|t| t.state == TState::Running) => {
                        self.act(Act::Cancel)
                    }
                    ESC => self.x.hide(),
                    DELETE => self.act(Act::ClearDone),
                    0x63 => {
                        if let Some(id) = self.transfers.get(self.tsel).map(|t| t.id) {
                            self.act(Act::CopyUrl(id));
                        }
                    }
                    _ => {}
                }
            }
            Tab::Backup => {
                let n = self.cfg.folders.len();
                match sym {
                    UP => self.bsel = self.bsel.saturating_sub(1),
                    DOWN if n > 0 => self.bsel = (self.bsel + 1).min(n - 1),
                    0x61 | 0x41 => self.act(Act::AddFolder),
                    DELETE => self.act(Act::RemoveFolder),
                    0x62 | 0x42 => self.act(Act::BackupNow),
                    0x70 | 0x50 => self.act(Act::Pause),
                    0x73 | 0x53 => self.act(Act::CycleEvery),
                    0x6f | 0x4f => self.act(Act::ToggleBattery),
                    0x65 | 0x45 => self.act(Act::EditConfig),
                    F5 => self.reload_config(),
                    ENTER | KP_ENTER if !signed_in => self.act(Act::SignIn),
                    ESC if self.backup_id.is_some() => self.act(Act::Cancel),
                    ESC => self.x.hide(),
                    _ => {}
                }
            }
        }
    }

    fn take_files_backup(&mut self, p: PathBuf) {
        let t = self.tab;
        self.tab = Tab::Backup;
        self.take_files(vec![p]);
        self.tab = t;
    }
}

/// The names in a Drive path: ["my-files", "NIRUX", "othala"], unescaped.
fn split_path(p: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut esc = false;
    for c in p.chars() {
        if esc {
            cur.push(c);
            esc = false;
        } else if c == '\\' {
            esc = true;
        } else if c == '/' {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
        } else {
            cur.push(c);
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn expand_tilde(s: &str) -> String {
    match s.strip_prefix("~/") {
        Some(r) => theme::home().join(r).display().to_string(),
        None if s == "~" => theme::home().display().to_string(),
        None => s.to_string(),
    }
}

/// Tab completion of a folder path: the longest common start of the
/// folders that begin with what is typed.
fn complete(s: &str) -> String {
    let full = expand_tilde(s);
    let (dir, start) = match full.rfind('/') {
        Some(i) => (&full[..=i], &full[i + 1..]),
        None => return s.to_string(),
    };
    let Ok(rd) = std::fs::read_dir(dir) else {
        return s.to_string();
    };
    let mut names: Vec<String> = rd
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| e.file_name().to_str().map(str::to_string))
        .filter(|n| n.starts_with(start) && (start.starts_with('.') || !n.starts_with('.')))
        .collect();
    names.sort();
    let Some(first) = names.first() else {
        return s.to_string();
    };
    let mut common = first.clone();
    for n in &names[1..] {
        while !n.starts_with(&common) {
            common.pop();
        }
    }
    let mut out = format!("{dir}{common}");
    if names.len() == 1 {
        out.push('/');
    }
    let home = theme::home().display().to_string();
    match out.strip_prefix(&home) {
        Some(r) if s.starts_with('~') => format!("~{r}"),
        _ => out,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drive_paths_split_into_names() {
        assert_eq!(
            split_path("/my-files/NIRUX/a\\/b"),
            vec!["my-files", "NIRUX", "a/b"]
        );
        assert_eq!(shown("/my-files/NIRUX/Uploads"), "My files/NIRUX/Uploads");
    }
}
