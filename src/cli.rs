//! The Proton Drive CLI, Proton's own program, does the sign-in, the
//! encryption and the talking to Proton. niruprodrive only ever runs the
//! commands listed in [`Cmd`]: there is no way here to trash, delete, move,
//! rename or replace anything in the Drive, and nothing overwrites a local
//! file either.
use serde_json::Value;
use std::io::{BufRead, BufReader, Read};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

/// What happens to a file that is already in the Drive under the same name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Same {
    /// A new revision of it: the Drive keeps the old one in its history.
    /// Used in niruprodrive's own folder (backups, Uploads).
    Revision,
    /// The upload gets a new name: used everywhere else.
    Rename,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Cmd {
    Login,
    Logout,
    List(String),
    Info(String),
    Size(String),
    MakeFolder {
        parent: String,
        name: String,
    },
    Upload {
        locals: Vec<PathBuf>,
        parent: String,
        same: Same,
    },
    Download {
        remotes: Vec<String>,
        to: PathBuf,
    },
    SetUrl {
        path: String,
        expires: Option<String>,
    },
    RemoveUrl(String),
}

/// A path is inside `base` (the same folder or below it).
pub fn within(path: &str, base: &str) -> bool {
    path == base || path.starts_with(&format!("{base}/"))
}

/// One name as a part of a Drive path: `/` in names is written `\/`.
pub fn escape(name: &str) -> String {
    name.replace('/', "\\/")
}

impl Cmd {
    pub fn args(&self) -> Vec<String> {
        let s = |x: &str| x.to_string();
        let mut a: Vec<String> = match self {
            Cmd::Login => vec![s("auth"), s("login")],
            Cmd::Logout => vec![s("auth"), s("logout")],
            Cmd::List(p) => vec![s("filesystem"), s("list"), p.clone()],
            Cmd::Info(p) => vec![s("filesystem"), s("info"), p.clone()],
            Cmd::Size(p) => vec![s("filesystem"), s("size"), p.clone()],
            Cmd::MakeFolder { parent, name } => {
                vec![
                    s("filesystem"),
                    s("create-folder"),
                    parent.clone(),
                    name.clone(),
                ]
            }
            Cmd::Upload {
                locals,
                parent,
                same,
            } => {
                let mut v = vec![
                    s("filesystem"),
                    s("upload"),
                    s("-f"),
                    s(match same {
                        Same::Revision => "create-new-revision",
                        Same::Rename => "rename",
                    }),
                    s("-d"),
                    s("merge"),
                ];
                v.extend(locals.iter().map(|p| p.display().to_string()));
                v.push(parent.clone());
                v
            }
            Cmd::Download { remotes, to } => {
                let mut v = vec![
                    s("filesystem"),
                    s("download"),
                    s("-f"),
                    s("rename"),
                    s("-d"),
                    s("merge"),
                ];
                v.extend(remotes.iter().cloned());
                v.push(to.display().to_string());
                v
            }
            Cmd::SetUrl { path, expires } => {
                let mut v = vec![s("sharing"), s("set-url"), s("--role"), s("viewer")];
                if let Some(e) = expires {
                    v.push(s("--expiration"));
                    v.push(e.clone());
                }
                v.push(path.clone());
                v
            }
            Cmd::RemoveUrl(p) => vec![s("sharing"), s("remove-url"), p.clone()],
        };
        a.push(s("--json"));
        a
    }

    /// The rule for where each command may act. `base` is niruprodrive's
    /// own folder (`/my-files/NIRUX`): new revisions and new folders only
    /// there; uploads that never replace anything (`Rename`) and links
    /// anywhere in My files.
    pub fn check(&self, base: &str) -> Result<(), String> {
        let mine = |p: &str| within(p, "/my-files");
        let ok = match self {
            Cmd::MakeFolder { parent, name } => {
                within(parent, "/my-files")
                    && (within(&format!("{parent}/{}", escape(name)), base))
                    && !name.is_empty()
                    && name != "."
                    && name != ".."
            }
            Cmd::Upload {
                parent,
                same: Same::Revision,
                locals,
            } => within(parent, base) && !locals.is_empty(),
            Cmd::Upload {
                parent,
                same: Same::Rename,
                locals,
            } => mine(parent) && !locals.is_empty(),
            Cmd::SetUrl { path, .. } | Cmd::RemoveUrl(path) => mine(path) && path != "/my-files",
            Cmd::Download { remotes, .. } => !remotes.is_empty(),
            _ => true,
        };
        if ok && !self.paths().iter().any(|p| p.split('/').any(|x| x == "..")) {
            Ok(())
        } else {
            Err(format!(
                "refused: {:?} is outside what niruprodrive may do",
                self
            ))
        }
    }

    fn paths(&self) -> Vec<&str> {
        match self {
            Cmd::List(p) | Cmd::Info(p) | Cmd::Size(p) | Cmd::RemoveUrl(p) => vec![p],
            Cmd::MakeFolder { parent, .. } | Cmd::Upload { parent, .. } => vec![parent],
            Cmd::SetUrl { path, .. } => vec![path],
            Cmd::Download { remotes, .. } => remotes.iter().map(String::as_str).collect(),
            _ => vec![],
        }
    }
}

pub struct Output {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl Output {
    pub fn ok(&self) -> bool {
        self.code == 0
    }

    pub fn json(&self) -> Option<Value> {
        let t = self.stdout.trim();
        serde_json::from_str(t).ok().or_else(|| {
            // A summary after other lines: the last JSON value printed.
            let i = t.rfind("\n{")?;
            serde_json::from_str(&t[i + 1..]).ok()
        })
    }

    /// What went wrong, in one line.
    pub fn error(&self) -> String {
        let clean = strip_ansi(&self.stderr);
        let line = clean
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with("at "))
            .find(|l| l.contains("rror") || l.contains("need") || l.contains("failed"))
            .or_else(|| clean.lines().map(str::trim).find(|l| !l.is_empty()))
            .unwrap_or("")
            .trim_start_matches("Error:")
            .trim()
            .to_string();
        if line.is_empty() {
            format!("the Proton Drive CLI stopped (exit {})", self.code)
        } else {
            line
        }
    }

    pub fn signed_out(&self) -> bool {
        let e = self.stderr.to_lowercase();
        e.contains("login first") || e.contains("not logged in") || e.contains("auth login")
    }
}

pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\u{1b}' {
            if it.peek() == Some(&'[') {
                it.next();
                for d in it.by_ref() {
                    if d.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        if c != '\r' {
            out.push(c);
        }
    }
    out
}

/// The CLI the installer put in place, or one on PATH.
pub fn find(configured: Option<&Path>) -> Option<PathBuf> {
    if let Some(p) = configured {
        return p.is_file().then(|| p.to_path_buf());
    }
    if let Some(p) = std::env::var_os("NIRUPRODRIVE_CLI") {
        return Some(PathBuf::from(p));
    }
    let lib = crate::theme::home().join(".local/lib/niruprodrive/proton-drive");
    if lib.is_file() {
        return Some(lib);
    }
    std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .map(|d| d.join("proton-drive"))
            .find(|p| p.is_file())
    })
}

/// Runs commands one at a time; `cancel` stops the one running.
#[derive(Clone)]
pub struct Runner {
    pub bin: PathBuf,
    pub credentials: String,
    pub base: String,
    running: Arc<AtomicI32>,
}

impl Runner {
    pub fn new(bin: PathBuf, credentials: &str, base: &str) -> Self {
        Self {
            bin,
            credentials: credentials.into(),
            base: base.into(),
            running: Arc::new(AtomicI32::new(0)),
        }
    }

    fn command(&self, cmd: &Cmd) -> Command {
        let mut c = Command::new(&self.bin);
        // A folder that is sure to exist: the CLI refuses to run in a
        // deleted one (the service may have been started from an
        // installer's temporary folder).
        c.args(cmd.args())
            .current_dir(crate::theme::home())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if self.credentials == "pass" {
            c.env("PROTON_DRIVE_CREDENTIALS_STORE", "pass");
        }
        // Its own process group, so a cancel stops it with its children,
        // and it ends with niruprodrive.
        unsafe {
            c.pre_exec(|| {
                libc::setpgid(0, 0);
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                Ok(())
            });
        }
        c
    }

    pub fn run(&self, cmd: &Cmd) -> Output {
        if let Err(e) = cmd.check(&self.base) {
            return Output {
                code: 126,
                stdout: String::new(),
                stderr: e,
            };
        }
        let mut child = match self.command(cmd).spawn() {
            Ok(c) => c,
            Err(e) => {
                return Output {
                    code: 127,
                    stdout: String::new(),
                    stderr: format!("cannot start {}: {e}", self.bin.display()),
                }
            }
        };
        self.running.store(child.id() as i32, Ordering::SeqCst);
        // Both pipes are read at once, so a chatty stderr cannot block it.
        let mut err = child.stderr.take();
        let errs = std::thread::spawn(move || {
            let mut s = String::new();
            if let Some(e) = err.as_mut() {
                let _ = e.read_to_string(&mut s);
            }
            s
        });
        let mut stdout = String::new();
        if let Some(mut o) = child.stdout.take() {
            let _ = o.read_to_string(&mut stdout);
        }
        let status = child.wait();
        self.running.store(0, Ordering::SeqCst);
        let stderr = errs.join().unwrap_or_default();
        let code = status
            .ok()
            .and_then(|s| {
                use std::os::unix::process::ExitStatusExt;
                s.code().or(s.signal().map(|n| 128 + n))
            })
            .unwrap_or(1);
        Output {
            code,
            stdout,
            stderr,
        }
    }

    /// Sign in: `url` gets the address of Proton's sign-in page as soon as
    /// the CLI prints it; the call returns when the sign-in is done.
    pub fn login(&self, mut url: impl FnMut(String)) -> Output {
        let mut child = match self.command(&Cmd::Login).spawn() {
            Ok(c) => c,
            Err(e) => {
                return Output {
                    code: 127,
                    stdout: String::new(),
                    stderr: format!("cannot start {}: {e}", self.bin.display()),
                }
            }
        };
        self.running.store(child.id() as i32, Ordering::SeqCst);
        let mut err = child.stderr.take();
        let errs = std::thread::spawn(move || {
            let mut s = String::new();
            if let Some(e) = err.as_mut() {
                let _ = e.read_to_string(&mut s);
            }
            s
        });
        let mut stdout = String::new();
        if let Some(o) = child.stdout.take() {
            for line in BufReader::new(o).lines().map_while(Result::ok) {
                if let Ok(v) = serde_json::from_str::<Value>(line.trim()) {
                    if let Some(u) = v["signInUrl"].as_str() {
                        url(u.to_string());
                    }
                } else if let Some(i) = line.find("https://") {
                    url(line[i..]
                        .split_whitespace()
                        .next()
                        .unwrap_or("")
                        .to_string());
                }
                stdout.push_str(&line);
                stdout.push('\n');
            }
        }
        let status = child.wait();
        self.running.store(0, Ordering::SeqCst);
        Output {
            code: status.ok().and_then(|s| s.code()).unwrap_or(1),
            stdout,
            stderr: errs.join().unwrap_or_default(),
        }
    }

    pub fn cancel(&self) {
        let pid = self.running.load(Ordering::SeqCst);
        if pid > 0 {
            unsafe { libc::kill(-pid, libc::SIGTERM) };
        }
    }
}

/// One file or folder in the Drive.
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub name: String,
    pub folder: bool,
    pub size: u64,
    /// "2026-11-24T10:04:00.000Z" shortened to "2026-11-24 10:04".
    pub modified: String,
    pub link: bool,
}

pub fn short_time(iso: &str) -> String {
    let t = iso.replace('T', " ");
    t.get(..16).unwrap_or(&t).to_string()
}

impl Node {
    pub fn from_json(v: &Value) -> Option<Node> {
        let name = match &v["name"] {
            Value::String(s) => s.clone(),
            Value::Object(o) => o.get("value")?.as_str()?.to_string(),
            _ => return None,
        };
        let kind = v["type"].as_str().unwrap_or("file");
        let folder = kind == "folder" || kind == "album";
        let rev = &v["activeRevision"];
        let size = rev["claimedSize"]
            .as_u64()
            .or_else(|| rev["storageSize"].as_u64())
            .or_else(|| v["totalStorageSize"].as_u64())
            .unwrap_or(0);
        let modified = rev["claimedModificationTime"]
            .as_str()
            .or_else(|| v["modificationTime"].as_str())
            .or_else(|| v["creationTime"].as_str())
            .map(short_time)
            .unwrap_or_default();
        Some(Node {
            name,
            folder,
            size,
            modified,
            link: v["isSharedByUrl"].as_bool().unwrap_or(false),
        })
    }
}

/// The nodes of a `filesystem list --json`.
pub fn parse_list(out: &Output) -> Vec<Node> {
    let Some(Value::Array(a)) = out.json() else {
        return Vec::new();
    };
    let mut v: Vec<Node> = a.iter().filter_map(Node::from_json).collect();
    v.sort_by(|a, b| {
        b.folder
            .cmp(&a.folder)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    v
}

/// The counts of an upload or download summary.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Summary {
    pub items: u64,
    pub bytes: u64,
    pub skipped: u64,
    pub failed: Vec<(String, String)>,
}

pub fn parse_summary(out: &Output) -> Option<Summary> {
    let v = out.json()?;
    if !v.is_object() || v.get("transferredItems").is_none() && v.get("failedItems").is_none() {
        return None;
    }
    Some(Summary {
        items: v["transferredItems"].as_u64().unwrap_or(0),
        bytes: v["transferredBytes"].as_u64().unwrap_or(0),
        skipped: v["skippedItems"].as_u64().unwrap_or(0),
        failed: v["failures"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|f| {
                        let err = match &f["error"] {
                            Value::String(s) => s.clone(),
                            Value::Object(o) => o
                                .get("message")
                                .and_then(|m| m.as_str())
                                .unwrap_or("failed")
                                .to_string(),
                            _ => "failed".into(),
                        };
                        (f["name"].as_str().unwrap_or("").to_string(), err)
                    })
                    .collect()
            })
            .unwrap_or_default(),
    })
}

/// The address of a public link from `sharing set-url` or `sharing status`.
pub fn parse_url(out: &Output) -> Option<String> {
    let v = out.json()?;
    v["urlAccess"]["url"]
        .as_str()
        .or_else(|| v["url"].as_str())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "/my-files/NIRUX";

    #[test]
    fn nothing_that_removes_or_replaces_can_be_asked_for() {
        // Every command line niruprodrive can make, with the words that
        // would trash, delete, move or overwrite nowhere in it.
        let cmds = [
            Cmd::Upload {
                locals: vec!["/home/u/a".into()],
                parent: format!("{BASE}/othala"),
                same: Same::Revision,
            },
            Cmd::Upload {
                locals: vec!["/home/u/a".into()],
                parent: "/my-files/Photos".into(),
                same: Same::Rename,
            },
            Cmd::Download {
                remotes: vec!["/my-files/a".into()],
                to: "/tmp".into(),
            },
            Cmd::MakeFolder {
                parent: BASE.into(),
                name: "othala".into(),
            },
            Cmd::SetUrl {
                path: "/my-files/a".into(),
                expires: Some("2026-12-01".into()),
            },
            Cmd::RemoveUrl("/my-files/a".into()),
            Cmd::List("/".into()),
        ];
        for c in &cmds {
            let a = c.args();
            for bad in [
                "trash",
                "delete",
                "move",
                "rename",
                "replace",
                "remove",
                "empty-trash",
            ] {
                let hit = a.iter().any(|x| x == bad);
                // "rename" is only ever the name strategy of a new upload or
                // download; remove-url (a link taken away) is a word of its own.
                let allowed = bad == "rename" && a.contains(&"-f".to_string());
                assert!(!hit || allowed, "{bad} in {a:?}");
            }
            assert_eq!(a.last().map(String::as_str), Some("--json"));
        }
        // Downloads never replace a local file.
        let d = cmds[2].args();
        assert_eq!(&d[2..6], ["-f", "rename", "-d", "merge"]);
    }

    #[test]
    fn new_revisions_and_folders_only_in_niruprodrives_own_folder() {
        let rev = |p: &str| Cmd::Upload {
            locals: vec!["/x".into()],
            parent: p.into(),
            same: Same::Revision,
        };
        assert!(rev("/my-files/NIRUX/othala/Documents").check(BASE).is_ok());
        assert!(rev("/my-files/NIRUX").check(BASE).is_ok());
        assert!(rev("/my-files/NIRUXX").check(BASE).is_err());
        assert!(rev("/my-files/Work").check(BASE).is_err());
        assert!(rev("/my-files/NIRUX/../Work").check(BASE).is_err());
        let new = |p: &str| Cmd::Upload {
            locals: vec!["/x".into()],
            parent: p.into(),
            same: Same::Rename,
        };
        assert!(new("/my-files/Work").check(BASE).is_ok());
        assert!(new("/shared-with-me/X").check(BASE).is_err());
        let mk = |p: &str, n: &str| Cmd::MakeFolder {
            parent: p.into(),
            name: n.into(),
        };
        assert!(mk("/my-files", "NIRUX").check(BASE).is_ok());
        assert!(mk("/my-files/NIRUX", "void").check(BASE).is_ok());
        assert!(mk("/my-files", "Other").check(BASE).is_err());
        assert!(mk("/my-files/NIRUX", "..").check(BASE).is_err());
        assert!(Cmd::SetUrl {
            path: "/my-files".into(),
            expires: None
        }
        .check(BASE)
        .is_err());
    }

    #[test]
    fn a_listing_becomes_nodes() {
        let out = Output {
            code: 0,
            stdout: r#"[
{"uid":"a","name":{"ok":true,"value":"Photos"},"type":"folder","modificationTime":"2026-11-20T08:00:00.000Z","isSharedByUrl":false},
{"uid":"b","name":{"ok":true,"value":"b.txt"},"type":"file","activeRevision":{"claimedSize":1234,"claimedModificationTime":"2026-11-21T09:30:12.000Z"},"isSharedByUrl":true},
{"uid":"c","name":{"ok":false,"error":{}},"type":"file"},
{"uid":"d","name":{"ok":true,"value":"A.txt"},"type":"file","totalStorageSize":99}
]"#
            .into(),
            stderr: String::new(),
        };
        let n = parse_list(&out);
        assert_eq!(n.len(), 3);
        assert_eq!(n[0].name, "Photos");
        assert!(n[0].folder);
        assert_eq!(n[1].name, "A.txt");
        assert_eq!(n[1].size, 99);
        assert_eq!(n[2].size, 1234);
        assert!(n[2].link);
        assert_eq!(n[2].modified, "2026-11-21 09:30");
    }

    #[test]
    fn summaries_links_and_errors() {
        let out = Output {
            code: 1,
            stdout: "uploading\n{\"transferredItems\":3,\"transferredBytes\":300,\"skippedItems\":1,\"failedItems\":1,\"failures\":[{\"name\":\"x\",\"error\":\"too big\"}]}".into(),
            stderr: "\u{1b}[31mError: 1 item(s) failed to upload\u{1b}[0m\n".into(),
        };
        let s = parse_summary(&out).unwrap();
        assert_eq!((s.items, s.bytes, s.skipped), (3, 300, 1));
        assert_eq!(s.failed, vec![("x".to_string(), "too big".to_string())]);
        assert_eq!(out.error(), "1 item(s) failed to upload");
        let l = Output {
            code: 0,
            stdout: r#"{"members":[],"urlAccess":{"url":"https://drive.proton.me/urls/ABC#key"}}"#
                .into(),
            stderr: String::new(),
        };
        assert_eq!(
            parse_url(&l).as_deref(),
            Some("https://drive.proton.me/urls/ABC#key")
        );
        let o = Output {
            code: 1,
            stdout: String::new(),
            stderr: "You need to login first\n".into(),
        };
        assert!(o.signed_out());
        assert_eq!(escape("a/b"), "a\\/b");
    }
}
