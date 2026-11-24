//! X11: the window, the icon in the bar's tray (XEmbed), the pop-up menu,
//! the clipboard for links, and files dropped on the window (XDND).
use crate::canvas::Canvas;
use x11rb::connection::{Connection, RequestConnection};
use x11rb::protocol::xproto::*;
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

pub type Res<T> = Result<T, Box<dyn std::error::Error>>;

pub struct Atoms {
    pub wm_protocols: Atom,
    pub wm_delete: Atom,
    pub net_wm_name: Atom,
    pub net_wm_icon: Atom,
    pub net_wm_pid: Atom,
    pub net_wm_window_type: Atom,
    pub net_wm_window_type_popup: Atom,
    pub utf8: Atom,
    pub niruwm_theme: Atom,
    pub clipboard: Atom,
    pub targets: Atom,
    pub text: Atom,
    pub tray: Atom,
    pub tray_opcode: Atom,
    pub manager: Atom,
    pub xembed_info: Atom,
    pub uri_list: Atom,
    pub xdnd_aware: Atom,
    pub xdnd_enter: Atom,
    pub xdnd_position: Atom,
    pub xdnd_status: Atom,
    pub xdnd_leave: Atom,
    pub xdnd_drop: Atom,
    pub xdnd_finished: Atom,
    pub xdnd_selection: Atom,
    pub xdnd_type_list: Atom,
    pub xdnd_copy: Atom,
    pub dnd_prop: Atom,
}

pub struct X {
    pub c: RustConnection,
    pub root: Window,
    pub screen: usize,
    pub depth: u8,
    gc: Gcontext,
    pub atoms: Atoms,
    /// The main window (unmapped while closed: it also owns the clipboard).
    pub win: Window,
    pub mapped: bool,
    /// The tray icon and the bar that holds it.
    pub tray: Window,
    pub docked: bool,
    pub menu: Window,
    min_keycode: u8,
    per: u8,
    keysyms: Vec<u32>,
}

fn atom(c: &RustConnection, name: &str) -> Res<Atom> {
    Ok(c.intern_atom(false, name.as_bytes())?.reply()?.atom)
}

impl X {
    pub fn open() -> Res<Self> {
        let (c, screen) = x11rb::connect(None)?;
        let s = &c.setup().roots[screen];
        let (root, depth, visual) = (s.root, s.root_depth, s.root_visual);
        let atoms = Atoms {
            wm_protocols: atom(&c, "WM_PROTOCOLS")?,
            wm_delete: atom(&c, "WM_DELETE_WINDOW")?,
            net_wm_name: atom(&c, "_NET_WM_NAME")?,
            net_wm_icon: atom(&c, "_NET_WM_ICON")?,
            net_wm_pid: atom(&c, "_NET_WM_PID")?,
            net_wm_window_type: atom(&c, "_NET_WM_WINDOW_TYPE")?,
            net_wm_window_type_popup: atom(&c, "_NET_WM_WINDOW_TYPE_POPUP_MENU")?,
            utf8: atom(&c, "UTF8_STRING")?,
            niruwm_theme: atom(&c, "_NIRUWM_THEME")?,
            clipboard: atom(&c, "CLIPBOARD")?,
            targets: atom(&c, "TARGETS")?,
            text: atom(&c, "TEXT")?,
            tray: atom(&c, &format!("_NET_SYSTEM_TRAY_S{screen}"))?,
            tray_opcode: atom(&c, "_NET_SYSTEM_TRAY_OPCODE")?,
            manager: atom(&c, "MANAGER")?,
            xembed_info: atom(&c, "_XEMBED_INFO")?,
            uri_list: atom(&c, "text/uri-list")?,
            xdnd_aware: atom(&c, "XdndAware")?,
            xdnd_enter: atom(&c, "XdndEnter")?,
            xdnd_position: atom(&c, "XdndPosition")?,
            xdnd_status: atom(&c, "XdndStatus")?,
            xdnd_leave: atom(&c, "XdndLeave")?,
            xdnd_drop: atom(&c, "XdndDrop")?,
            xdnd_finished: atom(&c, "XdndFinished")?,
            xdnd_selection: atom(&c, "XdndSelection")?,
            xdnd_type_list: atom(&c, "XdndTypeList")?,
            xdnd_copy: atom(&c, "XdndActionCopy")?,
            dnd_prop: atom(&c, "_NIRUPRODRIVE_DND")?,
        };
        let events = EventMask::EXPOSURE
            | EventMask::KEY_PRESS
            | EventMask::BUTTON_PRESS
            | EventMask::BUTTON_RELEASE
            | EventMask::POINTER_MOTION
            | EventMask::LEAVE_WINDOW
            | EventMask::STRUCTURE_NOTIFY;
        let win = c.generate_id()?;
        c.create_window(
            depth,
            win,
            root,
            0,
            0,
            820,
            540,
            0,
            WindowClass::INPUT_OUTPUT,
            visual,
            &CreateWindowAux::new()
                .background_pixel(s.black_pixel)
                .bit_gravity(Gravity::NORTH_WEST)
                .event_mask(events),
        )?;
        let tray = c.generate_id()?;
        c.create_window(
            depth,
            tray,
            root,
            0,
            0,
            18,
            18,
            0,
            WindowClass::INPUT_OUTPUT,
            visual,
            &CreateWindowAux::new()
                .background_pixel(s.black_pixel)
                .event_mask(
                    EventMask::EXPOSURE
                        | EventMask::BUTTON_PRESS
                        | EventMask::STRUCTURE_NOTIFY
                        | EventMask::ENTER_WINDOW
                        | EventMask::LEAVE_WINDOW,
                ),
        )?;
        let menu = c.generate_id()?;
        c.create_window(
            depth,
            menu,
            root,
            0,
            0,
            1,
            1,
            0,
            WindowClass::INPUT_OUTPUT,
            visual,
            &CreateWindowAux::new()
                .override_redirect(1)
                .save_under(1)
                .event_mask(
                    EventMask::EXPOSURE
                        | EventMask::BUTTON_PRESS
                        | EventMask::BUTTON_RELEASE
                        | EventMask::POINTER_MOTION
                        | EventMask::KEY_PRESS,
                ),
        )?;
        // The theme property on the root window, and a new tray (MANAGER).
        c.change_window_attributes(
            root,
            &ChangeWindowAttributesAux::new()
                .event_mask(EventMask::PROPERTY_CHANGE | EventMask::STRUCTURE_NOTIFY),
        )?;
        for w in [win, tray] {
            c.change_property8(
                PropMode::REPLACE,
                w,
                AtomEnum::WM_CLASS,
                AtomEnum::STRING,
                b"niruprodrive\0NIRUPRODRIVE\0",
            )?;
            c.change_property32(
                PropMode::REPLACE,
                w,
                atoms.net_wm_pid,
                AtomEnum::CARDINAL,
                &[std::process::id()],
            )?;
        }
        c.change_property32(
            PropMode::REPLACE,
            win,
            atoms.wm_protocols,
            AtomEnum::ATOM,
            &[atoms.wm_delete],
        )?;
        c.change_property32(
            PropMode::REPLACE,
            win,
            atoms.xdnd_aware,
            AtomEnum::ATOM,
            &[5],
        )?;
        c.change_property32(
            PropMode::REPLACE,
            tray,
            atoms.xembed_info,
            atoms.xembed_info,
            &[0, 1],
        )?;
        c.change_property32(
            PropMode::REPLACE,
            menu,
            atoms.net_wm_window_type,
            AtomEnum::ATOM,
            &[atoms.net_wm_window_type_popup],
        )?;
        for (w, t) in [(win, "niruprodrive"), (tray, "niruprodrive")] {
            c.change_property8(
                PropMode::REPLACE,
                w,
                atoms.net_wm_name,
                atoms.utf8,
                t.as_bytes(),
            )?;
            c.change_property8(
                PropMode::REPLACE,
                w,
                AtomEnum::WM_NAME,
                AtomEnum::STRING,
                t.as_bytes(),
            )?;
        }
        let gc = c.generate_id()?;
        c.create_gc(gc, win, &CreateGCAux::new().graphics_exposures(0))?;
        let setup = c.setup();
        let (min_keycode, max_keycode) = (setup.min_keycode, setup.max_keycode);
        let map = c
            .get_keyboard_mapping(min_keycode, max_keycode - min_keycode + 1)?
            .reply()?;
        let x = X {
            c,
            root,
            screen,
            depth,
            gc,
            atoms,
            win,
            mapped: false,
            tray,
            docked: false,
            menu,
            min_keycode,
            per: map.keysyms_per_keycode,
            keysyms: map.keysyms,
        };
        x.c.flush()?;
        Ok(x)
    }

    pub fn fd(&self) -> i32 {
        use std::os::fd::AsRawFd;
        self.c.stream().as_raw_fd()
    }

    pub fn poll(&self) -> Option<Event> {
        self.c.poll_for_event().ok().flatten()
    }

    pub fn flush(&self) {
        let _ = self.c.flush();
    }

    /// Ask the bar's tray to take the icon. False when there is no tray
    /// (yet: a MANAGER message says when one starts).
    pub fn dock(&mut self) -> bool {
        let owner = self
            .c
            .get_selection_owner(self.atoms.tray)
            .ok()
            .and_then(|c| c.reply().ok())
            .map(|r| r.owner)
            .unwrap_or(x11rb::NONE);
        if owner == x11rb::NONE {
            self.docked = false;
            return false;
        }
        let ev = ClientMessageEvent::new(
            32,
            owner,
            self.atoms.tray_opcode,
            [x11rb::CURRENT_TIME, 0, self.tray, 0, 0],
        );
        let _ = self.c.send_event(false, owner, EventMask::NO_EVENT, ev);
        self.flush();
        self.docked = true;
        true
    }

    pub fn tray_size(&self) -> (i32, i32) {
        self.c
            .get_geometry(self.tray)
            .ok()
            .and_then(|c| c.reply().ok())
            .map(|g| (g.width as i32, g.height as i32))
            .unwrap_or((18, 18))
    }

    pub fn show(&mut self) {
        let _ = self.c.map_window(self.win);
        let _ = self.c.configure_window(
            self.win,
            &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
        );
        // Ask the window manager to focus it (EWMH).
        if let Ok(a) = atom(&self.c, "_NET_ACTIVE_WINDOW") {
            let ev = ClientMessageEvent::new(32, self.win, a, [2, x11rb::CURRENT_TIME, 0, 0, 0]);
            let _ = self.c.send_event(
                false,
                self.root,
                EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
                ev,
            );
        }
        self.mapped = true;
        self.flush();
    }

    pub fn hide(&mut self) {
        let _ = self.c.unmap_window(self.win);
        self.mapped = false;
        self.flush();
    }

    pub fn set_icon(&self, sizes: &[(u32, u32, Vec<u32>)]) {
        let mut data = Vec::new();
        for (w, h, px) in sizes {
            data.push(*w);
            data.push(*h);
            data.extend_from_slice(px);
        }
        let _ = self.c.change_property32(
            PropMode::REPLACE,
            self.win,
            self.atoms.net_wm_icon,
            AtomEnum::CARDINAL,
            &data,
        );
    }

    /// Copy a canvas to a window, in slices that fit the request size.
    pub fn present(&self, w: Window, cv: &Canvas) {
        let bytes = cv.as_bytes();
        let max = self.c.maximum_request_bytes().saturating_sub(64).max(4096);
        let row_bytes = cv.w as usize * 4;
        let rows = (max / row_bytes.max(1)).max(1);
        let mut y = 0usize;
        while y < cv.h as usize {
            let n = rows.min(cv.h as usize - y);
            let _ = self.c.put_image(
                ImageFormat::Z_PIXMAP,
                w,
                self.gc,
                cv.w as u16,
                n as u16,
                0,
                y as i16,
                0,
                self.depth,
                &bytes[y * row_bytes..(y + n) * row_bytes],
            );
            y += n;
        }
        self.flush();
    }

    pub fn show_menu(&self, x: i32, y: i32, w: i32, h: i32) {
        let _ = self.c.configure_window(
            self.menu,
            &ConfigureWindowAux::new()
                .x(x)
                .y(y)
                .width(w as u32)
                .height(h as u32)
                .stack_mode(StackMode::ABOVE),
        );
        let _ = self.c.map_window(self.menu);
        self.flush();
        // Clicks outside close it: hold the pointer and keys while it is up.
        for _ in 0..20 {
            let g = self
                .c
                .grab_pointer(
                    true,
                    self.menu,
                    EventMask::BUTTON_PRESS | EventMask::BUTTON_RELEASE | EventMask::POINTER_MOTION,
                    GrabMode::ASYNC,
                    GrabMode::ASYNC,
                    x11rb::NONE,
                    x11rb::NONE,
                    x11rb::CURRENT_TIME,
                )
                .ok()
                .and_then(|c| c.reply().ok());
            if g.is_some_and(|r| r.status == GrabStatus::SUCCESS) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let _ = self.c.grab_keyboard(
            true,
            self.menu,
            x11rb::CURRENT_TIME,
            GrabMode::ASYNC,
            GrabMode::ASYNC,
        );
        self.flush();
    }

    pub fn hide_menu(&self) {
        let _ = self.c.ungrab_pointer(x11rb::CURRENT_TIME);
        let _ = self.c.ungrab_keyboard(x11rb::CURRENT_TIME);
        let _ = self.c.unmap_window(self.menu);
        self.flush();
    }

    pub fn screen_size(&self) -> (i32, i32) {
        let s = &self.c.setup().roots[self.screen];
        (s.width_in_pixels as i32, s.height_in_pixels as i32)
    }

    pub fn root_theme(&self) -> Option<String> {
        let r = self
            .c
            .get_property(
                false,
                self.root,
                self.atoms.niruwm_theme,
                AtomEnum::ANY,
                0,
                64,
            )
            .ok()?
            .reply()
            .ok()?;
        let s = String::from_utf8(r.value).ok()?;
        let s = s.trim_end_matches('\0').trim().to_string();
        (!s.is_empty()).then_some(s)
    }

    pub fn keysym(&self, keycode: u8, state: KeyButMask) -> u32 {
        let i = (keycode.saturating_sub(self.min_keycode)) as usize * self.per as usize;
        let shift = state.contains(KeyButMask::SHIFT);
        let get = |k: usize| self.keysyms.get(i + k).copied().unwrap_or(0);
        let low = get(0);
        let high = get(1);
        if shift && high != 0 {
            high
        } else if shift && (0x61..=0x7a).contains(&low) {
            low - 0x20
        } else {
            low
        }
    }

    // ------------------------------------------------------------ clipboard

    /// Own the clipboard (and the primary selection) with `text` in it.
    pub fn own_clipboard(&self) {
        for sel in [self.atoms.clipboard, AtomEnum::PRIMARY.into()] {
            let _ = self
                .c
                .set_selection_owner(self.win, sel, x11rb::CURRENT_TIME);
        }
        self.flush();
    }

    pub fn answer(&self, e: &SelectionRequestEvent, text: &str) {
        let a = &self.atoms;
        let prop = if e.property == x11rb::NONE {
            e.target
        } else {
            e.property
        };
        let ok = if e.target == a.targets {
            self.c
                .change_property32(
                    PropMode::REPLACE,
                    e.requestor,
                    prop,
                    AtomEnum::ATOM,
                    &[a.targets, a.utf8, AtomEnum::STRING.into(), a.text],
                )
                .is_ok()
        } else if e.target == a.utf8
            || e.target == u32::from(AtomEnum::STRING)
            || e.target == a.text
        {
            let kind = if e.target == a.utf8 {
                a.utf8
            } else {
                AtomEnum::STRING.into()
            };
            self.c
                .change_property8(PropMode::REPLACE, e.requestor, prop, kind, text.as_bytes())
                .is_ok()
        } else {
            false
        };
        let ev = SelectionNotifyEvent {
            response_type: SELECTION_NOTIFY_EVENT,
            sequence: 0,
            time: e.time,
            requestor: e.requestor,
            selection: e.selection,
            target: e.target,
            property: if ok { prop } else { x11rb::NONE },
        };
        let _ = self
            .c
            .send_event(false, e.requestor, EventMask::NO_EVENT, ev);
        self.flush();
    }

    // ------------------------------------------------------------ dropping

    pub fn send_client(&self, to: Window, kind: Atom, data: [u32; 5]) {
        let ev = ClientMessageEvent::new(32, to, kind, data);
        let _ = self.c.send_event(false, to, EventMask::NO_EVENT, ev);
        self.flush();
    }

    pub fn request_drop(&self, time: u32) {
        let _ = self.c.convert_selection(
            self.win,
            self.atoms.xdnd_selection,
            self.atoms.uri_list,
            self.atoms.dnd_prop,
            time,
        );
        self.flush();
    }

    pub fn take_drop(&self) -> Vec<u8> {
        self.c
            .get_property(
                true,
                self.win,
                self.atoms.dnd_prop,
                AtomEnum::ANY,
                0,
                1 << 24,
            )
            .ok()
            .and_then(|c| c.reply().ok())
            .map(|r| r.value)
            .unwrap_or_default()
    }

    pub fn offered_types(&self, source: Window, inline: &[u32], more: bool) -> Vec<u32> {
        if !more {
            return inline.iter().copied().filter(|x| *x != 0).collect();
        }
        self.c
            .get_property(
                false,
                source,
                self.atoms.xdnd_type_list,
                AtomEnum::ATOM,
                0,
                64,
            )
            .ok()
            .and_then(|c| c.reply().ok())
            .and_then(|r| r.value32().map(|i| i.collect()))
            .unwrap_or_default()
    }
}

/// Local paths from a text/uri-list.
pub fn parse_uris(data: &[u8]) -> Vec<std::path::PathBuf> {
    use std::os::unix::ffi::OsStrExt;
    let mut out = Vec::new();
    for line in data.split(|b| *b == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        let Some(rest) = line.strip_prefix(b"file://") else {
            continue;
        };
        // file://host/path: skip the host part.
        let path = match rest.iter().position(|b| *b == b'/') {
            Some(i) => &rest[i..],
            None => continue,
        };
        let mut bytes = Vec::with_capacity(path.len());
        let mut i = 0;
        while i < path.len() {
            if path[i] == b'%' && i + 2 < path.len() {
                if let Ok(v) =
                    u8::from_str_radix(std::str::from_utf8(&path[i + 1..i + 3]).unwrap_or(""), 16)
                {
                    bytes.push(v);
                    i += 3;
                    continue;
                }
            }
            bytes.push(path[i]);
            i += 1;
        }
        out.push(std::path::PathBuf::from(std::ffi::OsStr::from_bytes(
            &bytes,
        )));
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn uri_lists_become_paths() {
        let v = super::parse_uris(
            b"file:///home/u/a%20b.txt\r\nfile://host/x/%C3%B6\n# c\nhttp://x/y\n",
        );
        assert_eq!(
            v,
            vec![
                std::path::PathBuf::from("/home/u/a b.txt"),
                std::path::PathBuf::from("/x/ö")
            ]
        );
    }
}
