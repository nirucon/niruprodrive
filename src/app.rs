//! The running program: the tray icon, the window, the schedule and the
//! socket NIRUFM and the command line talk to. One thread draws and listens;
//! the CLI runs on the worker thread.
use crate::backup;
use crate::canvas::{Canvas, Rect};
use crate::cli::{self, Node, Runner, Same};
use crate::config::{self, tilde, Config};
use crate::icons::{self, Icons, Sign};
use crate::logo;
use crate::text::{Size, Text};
use crate::theme::{self, blend, Palette};
use crate::worker::{self, human, plural, Ev, Job, Shared};
use crate::x::{self, X};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};
use x11rb::protocol::xproto::{ConnectionExt as _, KeyButMask};
use x11rb::protocol::Event;

mod ui;

const DOUBLE_CLICK: Duration = Duration::from_millis(400);
/// A failed backup is tried again after this long.
const RETRY: i64 = 15 * 60;
/// On battery, look again after this long.
const BATTERY_WAIT: i64 = 10 * 60;

pub fn socket_path() -> PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(format!("/tmp/niruprodrive-{}", unsafe { libc::getuid() }))
        });
    dir.join("niruprodrive.sock")
}

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// "2026-11-24 12:03" in local time.
pub fn local_time(t: i64) -> String {
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let tt = t as libc::time_t;
    unsafe { libc::localtime_r(&tt, &mut tm) };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min
    )
}

/// The date `days` from now, as the CLI wants it: 2026-12-01.
pub fn date_in(days: i64) -> String {
    local_time(now() + days * 86400)[..10].to_string()
}

#[derive(Clone, Debug, PartialEq)]
enum Signed {
    Unknown,
    In,
    Out,
    /// The CLI is there but failed (network, keyring).
    Failed(String),
    NoCli,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tab {
    Drive,
    Transfers,
    Backup,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Backup,
    Upload,
    Download,
    Link,
    Unlink,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TState {
    Queued,
    Running,
    Done,
    Failed,
}

struct Transfer {
    id: u64,
    kind: Kind,
    title: String,
    state: TState,
    done: u64,
    total: u64,
    files: u64,
    of: u64,
    now: String,
    msg: String,
    url: Option<String>,
    when: i64,
    /// Asked for from NIRUFM or the command line: tell how it went.
    notify: bool,
}

#[derive(Clone, Debug, PartialEq)]
enum Act {
    Tab(Tab),
    SignIn,
    SignOut,
    CopyLoginUrl,
    OpenLoginUrl,
    Retry,
    Refresh,
    Go(String),
    Download,
    Link(Option<i64>),
    Unlink,
    BackupNow,
    Pause,
    Cancel,
    CycleEvery,
    ToggleBattery,
    AddFolder,
    RemoveFolder,
    EditConfig,
    ClearDone,
    CopyUrl(u64),
    Show,
    Quit,
    Yes,
    No,
}

#[derive(Clone, Debug, PartialEq)]
enum Hit {
    Act(Act),
    Row(usize),
    BRow(usize),
    TRow(usize),
}

struct Menu {
    items: Vec<(String, Act)>,
    hover: Option<usize>,
    w: i32,
    h: i32,
}

struct Confirm {
    text: String,
    act: Act,
}

struct M {
    s: f32,
    font: Size,
    small: Size,
    row_h: i32,
    pad: i32,
}

pub struct App {
    x: X,
    cv: Canvas,
    text: Text,
    p: Palette,
    theme: String,
    m: M,
    icons: Icons,
    cfg: Config,
    shared: Arc<Shared>,
    jobs: Sender<Job>,
    events: Receiver<Ev>,
    wake: i32,
    runner: Option<Runner>,
    listener: UnixListener,
    signed: Signed,
    login_url: Option<String>,
    logging_in: bool,
    tab: Tab,
    path: String,
    nodes: Vec<Node>,
    listing: Option<String>,
    list_error: Option<String>,
    cache: HashMap<String, Vec<Node>>,
    sel: usize,
    scroll: i32,
    transfers: Vec<Transfer>,
    tsel: usize,
    next_id: u64,
    backup_id: Option<u64>,
    paused: bool,
    battery_until: i64,
    usage: Option<u64>,
    bsel: usize,
    add_edit: Option<String>,
    confirm: Option<Confirm>,
    menu: Option<Menu>,
    clip: String,
    hits: Vec<(Rect, Hit)>,
    menu_hits: Vec<(Rect, usize)>,
    dirty: bool,
    tray_dirty: bool,
    tray_sign: Option<Sign>,
    cv_tray_size: (i32, i32),
    quit: bool,
    last_click: Option<(Instant, usize)>,
    drop_from: Option<(u32, bool)>,
    drop_hover: bool,
    message: Option<(String, Instant, bool)>,
}

fn scale_of(base: &Path) -> f32 {
    theme::configured_font_size(base)
        .map_or(1.0, |f| f / 13.0)
        .clamp(0.8, 2.5)
}

impl App {
    pub fn new(listener: UnixListener, show: bool) -> Result<Self, String> {
        let mut x = X::open().map_err(|e| format!("no X display: {e}"))?;
        let base = theme::config_home();
        let theme_name = x
            .root_theme()
            .unwrap_or_else(|| theme::configured_theme(&base));
        let p = theme::load(&base, &theme_name);
        let text = Text::new(theme::configured_font_path(&base).as_deref())?;
        let s = scale_of(&base);
        let font = text.size(13.0 * s);
        let small = text.size(11.0 * s);
        let m = M {
            s,
            font,
            small,
            row_h: font.height + (10.0 * s) as i32,
            pad: (8.0 * s) as i32,
        };
        let cfg = Config::load();
        let shared = Arc::new(Shared {
            state: std::sync::Mutex::new(backup::State::load()),
            cancel: std::sync::atomic::AtomicBool::new(false),
        });
        let (jtx, jrx) = std::sync::mpsc::channel();
        let (etx, erx) = std::sync::mpsc::channel();
        let wake = unsafe { libc::eventfd(0, libc::EFD_NONBLOCK | libc::EFD_CLOEXEC) };
        let runner = cli::find(cfg.cli.as_deref())
            .map(|b| Runner::new(b, &cfg.credentials, &cfg.remote_base()));
        let signed = if let Some(r) = &runner {
            let wfd = wake;
            worker::Worker::spawn(
                r.clone(),
                cfg.clone(),
                shared.clone(),
                jrx,
                etx,
                Box::new(move || {
                    let one: u64 = 1;
                    unsafe { libc::write(wfd, &one as *const u64 as *const libc::c_void, 8) };
                }),
            );
            let _ = jtx.send(Job::Check);
            Signed::Unknown
        } else {
            Signed::NoCli
        };
        x.set_icon(&icons::window_icons(&p));
        x.dock();
        if show {
            x.show();
        }
        listener.set_nonblocking(true).ok();
        let icons = Icons::new(&p);
        let path = cfg.remote_base();
        Ok(App {
            x,
            cv: Canvas::new(820, 540),
            text,
            p,
            theme: theme_name,
            m,
            icons,
            cfg,
            shared,
            jobs: jtx,
            events: erx,
            wake,
            runner,
            listener,
            signed,
            login_url: None,
            logging_in: false,
            tab: Tab::Drive,
            path,
            nodes: Vec::new(),
            listing: None,
            list_error: None,
            cache: HashMap::new(),
            sel: 0,
            scroll: 0,
            transfers: Vec::new(),
            tsel: 0,
            next_id: 1,
            backup_id: None,
            paused: false,
            battery_until: 0,
            usage: None,
            bsel: 0,
            add_edit: None,
            confirm: None,
            menu: None,
            clip: String::new(),
            hits: Vec::new(),
            menu_hits: Vec::new(),
            dirty: true,
            tray_dirty: true,
            tray_sign: None,
            cv_tray_size: (0, 0),
            quit: false,
            last_click: None,
            drop_from: None,
            drop_hover: false,
            message: None,
        })
    }

    // ------------------------------------------------------------ the loop

    /// Runs until Quit (0), the X display closing (0) or a stop signal
    /// (`guard::STOPPED_BY_SIGNAL`).
    pub fn run(&mut self) -> i32 {
        crate::guard::WAKE.store(self.wake, std::sync::atomic::Ordering::SeqCst);
        let mut code = 0;
        let xfd = self.x.fd();
        let lfd = {
            use std::os::fd::AsRawFd;
            self.listener.as_raw_fd()
        };
        while !self.quit {
            while let Some(ev) = self.x.poll() {
                self.event(ev);
            }
            while let Ok(e) = self.events.try_recv() {
                self.worker_event(e);
            }
            self.schedule();
            if self
                .message
                .as_ref()
                .is_some_and(|(_, t, _)| t.elapsed() > Duration::from_secs(4))
            {
                self.message = None;
                self.dirty = true;
            }
            if self.tray_dirty {
                self.draw_tray();
            }
            if self.dirty && self.x.mapped {
                self.draw();
                self.x.present(self.x.win, &self.cv);
                self.dirty = false;
            }
            if self.quit {
                break;
            }
            self.x.flush();
            let mut timeout: i64 = 60_000;
            if let Some(due) = self.due() {
                timeout = timeout.min(((due - now()).max(0) * 1000) + 50);
            }
            if let Some((_, t, _)) = &self.message {
                timeout = timeout.min(
                    Duration::from_secs(4)
                        .saturating_sub(t.elapsed())
                        .as_millis() as i64
                        + 10,
                );
            }
            // A signal that came before the wake-up was set up.
            if crate::guard::received().is_some() {
                timeout = 0;
            }
            let mut fds = [
                libc::pollfd {
                    fd: xfd,
                    events: libc::POLLIN,
                    revents: 0,
                },
                libc::pollfd {
                    fd: self.wake,
                    events: libc::POLLIN,
                    revents: 0,
                },
                libc::pollfd {
                    fd: lfd,
                    events: libc::POLLIN,
                    revents: 0,
                },
            ];
            unsafe { libc::poll(fds.as_mut_ptr(), 3, timeout.clamp(0, 600_000) as i32) };
            if let Some((sig, pid)) = crate::guard::received() {
                crate::guard::log(&format!(
                    "stopped by signal {}",
                    crate::guard::describe(sig, pid)
                ));
                code = crate::guard::STOPPED_BY_SIGNAL;
                break;
            }
            if fds[0].revents & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) != 0 {
                crate::guard::log("the X display closed (a logout); stopping");
                break;
            }
            if fds[1].revents & libc::POLLIN != 0 {
                let mut v = 0u64;
                unsafe { libc::read(self.wake, &mut v as *mut u64 as *mut libc::c_void, 8) };
            }
            if fds[2].revents & libc::POLLIN != 0 {
                self.accept();
            }
        }
        if let Some(r) = &self.runner {
            self.shared.cancel.store(true, Ordering::SeqCst);
            r.cancel();
        }
        if let Ok(st) = self.shared.state.lock() {
            let _ = st.save();
        }
        let _ = std::fs::remove_file(socket_path());
        code
    }

    fn event(&mut self, ev: Event) {
        match ev {
            // The bar shows the icon (or uncovers it): paint it again, even
            // when its state did not change. A first paint before the tray
            // mapped it is lost, and the window's black ground showed.
            Event::Expose(e) if e.window == self.x.tray => {
                self.tray_sign = None;
                self.tray_dirty = true;
            }
            Event::MapNotify(e) if e.window == self.x.tray => {
                self.tray_sign = None;
                self.tray_dirty = true;
            }
            Event::Expose(e) if e.window == self.x.menu => self.draw_menu(),
            Event::Expose(e) if e.count == 0 => self.dirty = true,
            Event::ConfigureNotify(e) if e.window == self.x.win => {
                if e.width as i32 != self.cv.w || e.height as i32 != self.cv.h {
                    self.cv.resize(e.width as i32, e.height as i32);
                    self.dirty = true;
                }
            }
            Event::ConfigureNotify(e) if e.window == self.x.tray => {
                self.tray_sign = None;
                self.tray_dirty = true;
            }
            Event::ReparentNotify(e) if e.window == self.x.tray => {
                // Back on the root window: the tray went away.
                if e.parent == self.x.root {
                    self.x.docked = false;
                    let _ = self.x.c.unmap_window(self.x.tray);
                }
            }
            Event::ClientMessage(e) => {
                let a = &self.x.atoms;
                let d = e.data.as_data32();
                if e.type_ == a.manager && d[1] == a.tray {
                    // A tray started (the bar came back): dock again.
                    self.x.dock();
                    self.tray_dirty = true;
                } else if e.type_ == a.wm_protocols && d[0] == a.wm_delete {
                    self.x.hide();
                } else if [a.xdnd_enter, a.xdnd_position, a.xdnd_leave, a.xdnd_drop]
                    .contains(&e.type_)
                {
                    self.xdnd(e.type_, d);
                }
            }
            Event::PropertyNotify(e) => {
                if e.window == self.x.root && e.atom == self.x.atoms.niruwm_theme {
                    self.reload_theme();
                }
            }
            Event::SelectionRequest(e) => {
                let clip = self.clip.clone();
                self.x.answer(&e, &clip);
            }
            Event::SelectionNotify(e) if e.selection == self.x.atoms.xdnd_selection => {
                self.dropped(e.property)
            }
            Event::ButtonPress(e) if e.event == self.x.tray => {
                let (rx, ry) = (e.root_x as i32, e.root_y as i32);
                match e.detail {
                    1 => self.toggle_window(),
                    3 => self.tray_menu(rx, ry),
                    _ => {}
                }
            }
            Event::ButtonPress(e) if self.menu.is_some() => {
                let hit = self
                    .menu_hits
                    .iter()
                    .find(|(r, _)| {
                        e.event == self.x.menu && r.contains(e.event_x as i32, e.event_y as i32)
                    })
                    .map(|(_, i)| *i);
                self.close_menu();
                if let Some(i) = hit {
                    let act = self.menu_act(i);
                    if let Some(a) = act {
                        self.act(a);
                    }
                }
            }
            Event::MotionNotify(e) if self.menu.is_some() => {
                let h = self
                    .menu_hits
                    .iter()
                    .find(|(r, _)| {
                        e.event == self.x.menu && r.contains(e.event_x as i32, e.event_y as i32)
                    })
                    .map(|(_, i)| *i);
                if let Some(m) = self.menu.as_mut() {
                    if m.hover != h {
                        m.hover = h;
                        self.draw_menu();
                    }
                }
            }
            Event::KeyPress(e) if self.menu.is_some() => {
                let sym = self.x.keysym(e.detail, e.state);
                self.menu_key(sym);
            }
            Event::ButtonPress(e) if e.event == self.x.win => self.click(
                e.detail,
                e.event_x as i32,
                e.event_y as i32,
                e.root_x as i32,
                e.root_y as i32,
            ),
            Event::KeyPress(e) if e.event == self.x.win => self.key(e.detail, e.state),
            _ => {}
        }
    }

    fn reload_theme(&mut self) {
        let base = theme::config_home();
        let name = self
            .x
            .root_theme()
            .unwrap_or_else(|| theme::configured_theme(&base));
        let p = theme::load(&base, &name);
        if p != self.p || name != self.theme {
            self.p = p;
            self.theme = name;
            self.icons = Icons::new(&p);
            self.x.set_icon(&icons::window_icons(&p));
            self.dirty = true;
            self.tray_dirty = true;
        }
    }

    fn toggle_window(&mut self) {
        if self.x.visible_here() {
            self.x.hide();
        } else {
            self.show_window();
        }
    }

    fn show_window(&mut self) {
        self.x.show();
        if self.signed == Signed::In
            && self.tab == Tab::Drive
            && self.nodes.is_empty()
            && self.listing.is_none()
        {
            let p = self.path.clone();
            self.go(p);
        }
        self.dirty = true;
    }

    fn flash(&mut self, s: impl Into<String>, error: bool) {
        self.message = Some((s.into(), Instant::now(), error));
        self.dirty = true;
    }

    fn send(&mut self, j: Job) {
        if self.runner.is_none() {
            self.flash(
                "The Proton Drive CLI is missing: run the installer again",
                true,
            );
            return;
        }
        let _ = self.jobs.send(j);
    }

    // ------------------------------------------------------------ worker

    fn worker_event(&mut self, e: Ev) {
        self.dirty = true;
        match e {
            Ev::Signed(r) => {
                let was = self.signed.clone();
                self.logging_in = false;
                self.login_url = None;
                self.signed = match r {
                    Ok(true) => Signed::In,
                    Ok(false) => Signed::Out,
                    Err(e) => Signed::Failed(e),
                };
                if self.signed == Signed::In && was != Signed::In {
                    let p = self.path.clone();
                    self.go(p);
                    self.send(Job::Usage);
                }
                if self.signed == Signed::Out && was == Signed::In {
                    notify("Proton Drive", "Signed out: sign in again in niruprodrive.");
                }
                self.tray_dirty = true;
            }
            Ev::LoginUrl(u) => {
                self.login_url = Some(u.clone());
                open_url(&u);
            }
            Ev::Listed { path, result } => {
                if self.listing.as_deref() == Some(path.as_str()) {
                    self.listing = None;
                }
                match result {
                    Ok(nodes) => {
                        self.cache.insert(path.clone(), nodes.clone());
                        if path == self.path {
                            self.nodes = nodes;
                            self.list_error = None;
                            self.sel = self.sel.min(self.nodes.len().saturating_sub(1));
                        }
                    }
                    Err(e) => {
                        // niruprodrive's own folder before anything was sent
                        // to it: empty, not an error.
                        let missing = e.to_lowercase().contains("not found");
                        if missing && cli::within(&path, &self.cfg.remote_base()) {
                            if path == self.path {
                                self.nodes.clear();
                                self.list_error = None;
                            }
                        } else if path == self.path {
                            self.list_error = Some(e);
                        }
                    }
                }
            }
            Ev::Usage(n) => self.usage = Some(n),
            Ev::Start { id } => {
                if let Some(t) = self.transfer(id) {
                    t.state = TState::Running;
                    t.when = now();
                }
                self.tray_dirty = true;
            }
            Ev::Progress {
                id,
                done,
                total,
                files,
                of,
                now,
            } => {
                if let Some(t) = self.transfer(id) {
                    t.done = done;
                    t.total = total;
                    t.files = files;
                    t.of = of;
                    t.now = now;
                }
            }
            Ev::Done { id, result, url } => self.finished(id, result, url),
        }
    }

    fn transfer(&mut self, id: u64) -> Option<&mut Transfer> {
        self.transfers.iter_mut().find(|t| t.id == id)
    }

    fn finished(&mut self, id: u64, result: Result<String, String>, url: Option<String>) {
        let Some(t) = self.transfers.iter_mut().find(|t| t.id == id) else {
            return;
        };
        t.when = now();
        t.url = url.clone();
        let (ok, msg) = match &result {
            Ok(m) => (true, m.clone()),
            Err(e) => (false, e.clone()),
        };
        t.state = if ok { TState::Done } else { TState::Failed };
        t.msg = msg.clone();
        let (kind, title, notify_it) = (t.kind, t.title.clone(), t.notify);
        if let Some(u) = url {
            self.copy(u);
        }
        if kind == Kind::Backup {
            self.backup_id = None;
            self.send(Job::Usage);
            if !ok && msg != "cancelled" {
                notify("Backup to Proton Drive failed", &msg);
            }
        }
        if notify_it {
            let head = match (kind, ok) {
                (Kind::Link, true) => "Link copied".to_string(),
                (Kind::Upload, true) => "Sent to Proton Drive".to_string(),
                (Kind::Download, true) => "Downloaded".to_string(),
                (_, true) => "Done".to_string(),
                (_, false) => "Proton Drive: failed".to_string(),
            };
            notify(&head, &format!("{title}: {msg}"));
        } else if self.x.mapped {
            self.flash(format!("{title}: {msg}"), !ok);
        }
        // A change in the folder shown: list it again.
        if ok
            && matches!(
                kind,
                Kind::Upload | Kind::Link | Kind::Unlink | Kind::Backup
            )
        {
            self.cache.clear();
            if self.signed == Signed::In && self.x.mapped && self.tab == Tab::Drive {
                let p = self.path.clone();
                self.list(p);
            }
        }
        self.transfers.truncate(200);
        self.tray_dirty = true;
    }

    fn queue(&mut self, kind: Kind, title: String, notify: bool) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.transfers.insert(
            0,
            Transfer {
                id,
                kind,
                title,
                state: TState::Queued,
                done: 0,
                total: 0,
                files: 0,
                of: 0,
                now: String::new(),
                msg: String::new(),
                url: None,
                when: now(),
                notify,
            },
        );
        self.tray_dirty = true;
        self.dirty = true;
        id
    }

    fn copy(&mut self, s: String) {
        self.clip = s;
        self.x.own_clipboard();
    }

    // ------------------------------------------------------------ schedule

    /// When the next backup is due (seconds since the epoch), if one is.
    fn due(&self) -> Option<i64> {
        let every = self.cfg.every.seconds()? as i64;
        if self.paused
            || self.signed != Signed::In
            || self.backup_id.is_some()
            || self.cfg.folders.is_empty()
        {
            return None;
        }
        let st = self.shared.state.lock().ok()?;
        let next = if st.last_run == 0 {
            now()
        } else if st.last_error.is_empty() {
            st.last_run + every
        } else {
            st.last_run + every.min(RETRY)
        };
        Some(next.max(self.battery_until))
    }

    fn schedule(&mut self) {
        let Some(due) = self.due() else { return };
        if now() < due {
            return;
        }
        if !self.cfg.on_battery && backup::on_battery() {
            self.battery_until = now() + BATTERY_WAIT;
            return;
        }
        self.start_backup(false);
    }

    fn start_backup(&mut self, notify: bool) {
        if self.backup_id.is_some() {
            self.flash("A backup is already running", false);
            return;
        }
        if self.cfg.folders.is_empty() {
            self.tab = Tab::Backup;
            self.flash("Add a folder to back up first", true);
            return;
        }
        if self.signed != Signed::In {
            self.flash("Sign in first", true);
            return;
        }
        let id = self.queue(Kind::Backup, format!("Backup of {}", self.cfg.host), notify);
        self.backup_id = Some(id);
        self.send(Job::Backup { id });
    }

    // ------------------------------------------------------------ socket

    fn accept(&mut self) {
        while let Ok((s, _)) = self.listener.accept() {
            let _ = s.set_nonblocking(false);
            let _ = s.set_read_timeout(Some(Duration::from_millis(300)));
            let _ = s.set_write_timeout(Some(Duration::from_millis(300)));
            let mut line = String::new();
            let mut r = BufReader::new(&s);
            if r.read_line(&mut line).is_err() {
                continue;
            }
            let reply = match serde_json::from_str::<Value>(line.trim()) {
                Ok(v) => self.request(&v),
                Err(_) => json!({"ok": false, "error": "not JSON"}),
            };
            let mut w = &s;
            let _ = writeln!(w, "{reply}");
        }
    }

    fn paths_of(v: &Value) -> Vec<PathBuf> {
        v["paths"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|p| p.as_str())
                    .map(PathBuf::from)
                    .filter(|p| p.is_absolute())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// One request from NIRUFM or the command line.
    fn request(&mut self, v: &Value) -> Value {
        let op = v["op"].as_str().unwrap_or("");
        match op {
            "ping" => json!({
                "ok": true,
                "version": env!("CARGO_PKG_VERSION"),
                "signed_in": self.signed == Signed::In,
                "roots": self.cfg.folders.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
            }),
            "status" => {
                let st = self.shared.state.lock().unwrap();
                let marks: Vec<&str> = Self::paths_of(v)
                    .iter()
                    .map(|p| backup::mark_of(&self.cfg, &st, p).name())
                    .collect();
                json!({"ok": true, "marks": marks})
            }
            "show" => {
                self.show_window();
                json!({"ok": true})
            }
            "upload" => {
                let paths: Vec<PathBuf> = Self::paths_of(v)
                    .into_iter()
                    .filter(|p| p.exists())
                    .collect();
                if paths.is_empty() {
                    return json!({"ok": false, "error": "nothing to send"});
                }
                if self.signed != Signed::In {
                    self.show_window();
                    return json!({"ok": false, "error": "not signed in"});
                }
                let parent = self.cfg.remote_uploads();
                let title = title_of(&paths);
                let id = self.queue(Kind::Upload, format!("{title} → {}", shown(&parent)), true);
                self.send(Job::Upload {
                    id,
                    locals: paths,
                    parent,
                    same: Same::Revision,
                });
                json!({"ok": true, "queued": id})
            }
            "link" => {
                let Some(p) = Self::paths_of(v).into_iter().next().filter(|p| p.exists()) else {
                    return json!({"ok": false, "error": "no such file"});
                };
                if self.signed != Signed::In {
                    self.show_window();
                    return json!({"ok": false, "error": "not signed in"});
                }
                let days = v["days"].as_i64().filter(|d| *d > 0);
                let id = self.queue(Kind::Link, format!("Link to {}", worker::name_of(&p)), true);
                self.send(Job::Link {
                    id,
                    remote: None,
                    local: Some(p),
                    expires: days.map(date_in),
                });
                json!({"ok": true, "queued": id})
            }
            "backup" => {
                self.start_backup(true);
                json!({"ok": self.backup_id.is_some()})
            }
            "state" => {
                let st = self.shared.state.lock().unwrap();
                json!({
                    "ok": true,
                    "signed": format!("{:?}", self.signed),
                    "running": self.transfers.iter().filter(|t| t.state == TState::Running).count(),
                    "queued": self.transfers.iter().filter(|t| t.state == TState::Queued).count(),
                    "last_run": st.last_run,
                    "last_ok": st.last_ok,
                    "last_error": st.last_error,
                    "folders": self.cfg.folders.iter().map(|p| tilde(p)).collect::<Vec<_>>(),
                    "remote": self.cfg.remote_host(),
                    "transfers": self.transfers.iter().take(20).map(|t| json!({
                        "title": t.title, "state": format!("{:?}", t.state), "msg": t.msg,
                    })).collect::<Vec<_>>(),
                })
            }
            "reload" => {
                self.reload_config();
                json!({"ok": true})
            }
            "quit" => {
                self.quit = true;
                json!({"ok": true})
            }
            _ => json!({"ok": false, "error": format!("unknown op {op}")}),
        }
    }

    fn reload_config(&mut self) {
        self.cfg = Config::load();
        self.send(Job::Settings(self.cfg.clone()));
        self.dirty = true;
    }

    fn save_config(&mut self) {
        if let Err(e) = self.cfg.save() {
            self.flash(format!("cannot save the settings: {e}"), true);
        }
        let _ = self.jobs.send(Job::Settings(self.cfg.clone()));
        self.dirty = true;
    }

    // ------------------------------------------------------------ drive

    fn list(&mut self, path: String) {
        if self.signed != Signed::In {
            return;
        }
        self.listing = Some(path.clone());
        self.send(Job::List(path));
    }

    fn go(&mut self, path: String) {
        if path != self.path {
            self.sel = 0;
            self.scroll = 0;
        }
        self.path = path.clone();
        self.list_error = None;
        self.nodes = self.cache.get(&path).cloned().unwrap_or_default();
        self.list(path);
        self.dirty = true;
    }

    fn node_path(&self, i: usize) -> Option<String> {
        self.nodes
            .get(i)
            .map(|n| format!("{}/{}", self.path, cli::escape(&n.name)))
    }

    fn open(&mut self, i: usize) {
        let Some(n) = self.nodes.get(i).cloned() else {
            return;
        };
        if n.folder {
            if let Some(p) = self.node_path(i) {
                self.go(p);
            }
        } else {
            self.act(Act::Download);
        }
    }

    fn up(&mut self) {
        if let Some((parent, _)) = self.path.rsplit_once('/') {
            if parent.len() >= "/my-files".len() && parent.starts_with("/my-files") {
                let parent = parent.to_string();
                let came = self.path.clone();
                self.go(parent);
                // Keep the folder we came from selected.
                if let Some(i) = self
                    .nodes
                    .iter()
                    .position(|n| came.ends_with(&format!("/{}", cli::escape(&n.name))))
                {
                    self.sel = i;
                }
            }
        }
    }

    /// Files dropped on the window: sent to the folder shown, or added to
    /// the backup on the Backup tab.
    fn take_files(&mut self, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }
        if self.tab == Tab::Backup {
            let mut added = 0;
            for p in paths.into_iter().filter(|p| p.is_dir()) {
                if !self.cfg.folders.iter().any(|f| p.starts_with(f)) {
                    self.cfg.folders.retain(|f| !f.starts_with(&p));
                    self.cfg.folders.push(p);
                    added += 1;
                }
            }
            if added > 0 {
                self.save_config();
                self.flash(
                    format!("{} added to the backup", plural(added, "folder")),
                    false,
                );
            } else {
                self.flash("Drop folders here (ones not backed up already)", true);
            }
            return;
        }
        if self.signed != Signed::In {
            self.flash("Sign in first", true);
            return;
        }
        let parent = self.path.clone();
        let same = if cli::within(&parent, &self.cfg.remote_base()) {
            Same::Revision
        } else {
            Same::Rename
        };
        let title = format!("{} → {}", title_of(&paths), shown(&parent));
        let id = self.queue(Kind::Upload, title, false);
        self.send(Job::Upload {
            id,
            locals: paths,
            parent,
            same,
        });
        self.flash("Sending: see Transfers", false);
    }

    // ------------------------------------------------------------ XDND

    fn xdnd(&mut self, t: u32, d: [u32; 5]) {
        let a = (
            self.x.atoms.xdnd_enter,
            self.x.atoms.xdnd_position,
            self.x.atoms.xdnd_leave,
            self.x.atoms.xdnd_drop,
            self.x.atoms.xdnd_status,
            self.x.atoms.xdnd_finished,
            self.x.atoms.xdnd_copy,
        );
        let win = self.x.win;
        if t == a.0 {
            let types = self.x.offered_types(d[0], &d[2..5], d[1] & 1 == 1);
            self.drop_from = Some((d[0], types.contains(&self.x.atoms.uri_list)));
        } else if t == a.1 {
            let Some((src, files)) = self.drop_from else {
                return;
            };
            let ok = files
                && (self.tab == Tab::Backup
                    || (self.tab == Tab::Drive && self.signed == Signed::In));
            if ok != self.drop_hover {
                self.drop_hover = ok;
                self.dirty = true;
            }
            self.x.send_client(
                src,
                a.4,
                [win, u32::from(ok), 0, 0, if ok { a.6 } else { 0 }],
            );
        } else if t == a.2 {
            self.drop_from = None;
            self.drop_hover = false;
            self.dirty = true;
        } else if t == a.3 {
            if self.drop_hover {
                self.x.request_drop(d[2]);
            } else if let Some((src, _)) = self.drop_from.take() {
                self.x.send_client(src, a.5, [win, 0, 0, 0, 0]);
            }
        }
    }

    fn dropped(&mut self, property: u32) {
        let from = self.drop_from.take();
        self.drop_hover = false;
        self.dirty = true;
        let data = if property != x11rb::NONE {
            self.x.take_drop()
        } else {
            Vec::new()
        };
        let paths = x::parse_uris(&data);
        let ok = !paths.is_empty();
        self.take_files(paths);
        if let Some((src, _)) = from {
            let action = if ok { self.x.atoms.xdnd_copy } else { 0 };
            self.x.send_client(
                src,
                self.x.atoms.xdnd_finished,
                [self.x.win, u32::from(ok), action, 0, 0],
            );
        }
    }

    // ------------------------------------------------------------ actions

    fn act(&mut self, a: Act) {
        self.dirty = true;
        match a {
            Act::Tab(t) => {
                self.tab = t;
                if t == Tab::Drive && self.nodes.is_empty() {
                    let p = self.path.clone();
                    self.go(p);
                }
            }
            Act::SignIn => {
                if self.runner.is_none() {
                    return self.flash("The Proton Drive CLI is missing", true);
                }
                if !self.logging_in {
                    self.logging_in = true;
                    self.login_url = None;
                    self.send(Job::Login);
                }
            }
            Act::SignOut => {
                self.confirm = Some(Confirm {
                    text: "Sign out of Proton Drive on this computer? Backups stop until you sign in again.".into(),
                    act: Act::SignOut,
                });
            }
            Act::CopyLoginUrl => {
                if let Some(u) = self.login_url.clone() {
                    self.copy(u);
                    self.flash("Sign-in address copied", false);
                }
            }
            Act::OpenLoginUrl => {
                if let Some(u) = &self.login_url {
                    open_url(u);
                }
            }
            Act::Retry => {
                self.signed = Signed::Unknown;
                self.send(Job::Check);
            }
            Act::Refresh => {
                let p = self.path.clone();
                self.cache.remove(&p);
                self.list(p);
                self.send(Job::Usage);
            }
            Act::Go(p) => self.go(p),
            Act::Download => {
                let Some(n) = self.nodes.get(self.sel).cloned() else {
                    return;
                };
                let Some(remote) = self.node_path(self.sel) else {
                    return;
                };
                let to = self.cfg.downloads.clone();
                let id = self.queue(
                    Kind::Download,
                    format!("{} → {}", n.name, tilde(&to)),
                    false,
                );
                self.send(Job::Download {
                    id,
                    remotes: vec![remote],
                    to,
                });
                self.flash("Downloading: see Transfers", false);
            }
            Act::Link(days) => {
                let Some(n) = self.nodes.get(self.sel).cloned() else {
                    return;
                };
                let remote = self.node_path(self.sel);
                let id = self.queue(Kind::Link, format!("Link to {}", n.name), false);
                self.send(Job::Link {
                    id,
                    remote,
                    local: None,
                    expires: days.map(date_in),
                });
            }
            Act::Unlink => {
                let Some(n) = self.nodes.get(self.sel).cloned() else {
                    return;
                };
                if !n.link {
                    return self.flash(format!("{} has no link", n.name), true);
                }
                self.confirm = Some(Confirm {
                    text: format!(
                        "Remove the public link to {}? Whoever has it can no longer open it.",
                        n.name
                    ),
                    act: Act::Unlink,
                });
            }
            Act::BackupNow => self.start_backup(false),
            Act::Pause => {
                self.paused = !self.paused;
                self.flash(
                    if self.paused {
                        "Backups paused"
                    } else {
                        "Backups resumed"
                    },
                    false,
                );
                self.tray_dirty = true;
            }
            Act::Cancel => {
                if let Some(r) = &self.runner {
                    if self.transfers.iter().any(|t| t.state == TState::Running) {
                        self.shared.cancel.store(true, Ordering::SeqCst);
                        r.cancel();
                        self.flash("Cancelling…", false);
                    }
                }
            }
            Act::CycleEvery => {
                self.cfg.every = self.cfg.every.next();
                self.save_config();
            }
            Act::ToggleBattery => {
                self.cfg.on_battery = !self.cfg.on_battery;
                self.battery_until = 0;
                self.save_config();
            }
            Act::AddFolder => {
                self.add_edit = Some("~/".into());
            }
            Act::RemoveFolder => {
                if let Some(f) = self.cfg.folders.get(self.bsel).cloned() {
                    self.confirm = Some(Confirm {
                        text: format!(
                            "Stop backing up {}? What is in the Drive stays there.",
                            tilde(&f)
                        ),
                        act: Act::RemoveFolder,
                    });
                }
            }
            Act::EditConfig => {
                if !config::path().exists() {
                    let _ = self.cfg.save();
                }
                open_url(&config::path().display().to_string());
                self.flash("Saved changes are read when you come back here (F5)", false);
            }
            Act::ClearDone => {
                self.transfers
                    .retain(|t| matches!(t.state, TState::Queued | TState::Running));
                self.tsel = 0;
            }
            Act::CopyUrl(id) => {
                if let Some(u) = self
                    .transfers
                    .iter()
                    .find(|t| t.id == id)
                    .and_then(|t| t.url.clone())
                {
                    self.copy(u);
                    self.flash("Link copied", false);
                }
            }
            // The tray menu's first row: hide when it is in view, else bring it here.
            Act::Show => self.toggle_window(),
            Act::Quit => {
                if self.transfers.iter().any(|t| t.state == TState::Running) {
                    self.confirm = Some(Confirm {
                        text: "Something is still being sent. Quit anyway? It is sent again next time.".into(),
                        act: Act::Quit,
                    });
                } else {
                    self.quit = true;
                }
            }
            Act::Yes => match self.confirm.take().map(|c| c.act) {
                Some(Act::SignOut) => {
                    self.send(Job::Logout);
                    self.nodes.clear();
                    self.cache.clear();
                }
                Some(Act::Unlink) => {
                    if let (Some(n), Some(remote)) =
                        (self.nodes.get(self.sel).cloned(), self.node_path(self.sel))
                    {
                        let id = self.queue(Kind::Unlink, format!("Link to {}", n.name), false);
                        self.send(Job::RemoveLink { id, remote });
                    }
                }
                Some(Act::RemoveFolder) => {
                    if self.bsel < self.cfg.folders.len() {
                        self.cfg.folders.remove(self.bsel);
                        self.bsel = self.bsel.min(self.cfg.folders.len().saturating_sub(1));
                        self.save_config();
                    }
                }
                Some(Act::Quit) => self.quit = true,
                _ => {}
            },
            Act::No => self.confirm = None,
        }
    }
}

/// Paths shown short: "a.txt", or "a.txt and 3 more".
fn title_of(paths: &[PathBuf]) -> String {
    let first = worker::name_of(&paths[0]);
    if paths.len() == 1 {
        first
    } else {
        format!("{first} and {} more", paths.len() - 1)
    }
}

/// A Drive path as people read it: "My files/NIRUX/Uploads".
pub fn shown(path: &str) -> String {
    let t = path.strip_prefix("/my-files").unwrap_or(path);
    format!("My files{}", t).replace("\\/", "/")
}

pub fn notify(title: &str, body: &str) {
    let (t, b) = (title.to_string(), body.to_string());
    std::thread::spawn(move || {
        let _ = std::process::Command::new("notify-send")
            .args(["-a", "niruprodrive", "-i", "niruprodrive", &t, &b])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    });
}

pub fn open_url(u: &str) {
    let u = u.to_string();
    std::thread::spawn(move || {
        let _ = std::process::Command::new("xdg-open")
            .arg(&u)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    });
}

/// Talk to the running niruprodrive: one JSON line each way.
pub fn ask(v: &Value) -> Result<Value, String> {
    let s = UnixStream::connect(socket_path())
        .map_err(|_| "niruprodrive is not running".to_string())?;
    s.set_read_timeout(Some(Duration::from_secs(5))).ok();
    let mut w = &s;
    writeln!(w, "{v}").map_err(|e| e.to_string())?;
    let mut line = String::new();
    BufReader::new(&s)
        .read_line(&mut line)
        .map_err(|e| e.to_string())?;
    serde_json::from_str(line.trim()).map_err(|e| e.to_string())
}
