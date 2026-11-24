//! The one thread that runs the Proton Drive CLI, one command at a time
//! (two at once could step on each other's cache). Quick jobs (a listing,
//! a link) go between the uploads of a long backup, so the window stays
//! useful while a backup runs.
use crate::backup::{self, Op, State};
use crate::cli::{self, Cmd, Node, Runner, Same};
use crate::config::Config;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub enum Job {
    Check,
    Login,
    Logout,
    List(String),
    Usage,
    Settings(Config),
    Backup {
        id: u64,
    },
    Upload {
        id: u64,
        locals: Vec<PathBuf>,
        parent: String,
        same: Same,
    },
    Download {
        id: u64,
        remotes: Vec<String>,
        to: PathBuf,
    },
    /// A link to a file in the Drive, or to a local file (sent first when
    /// it is not in the Drive as it is now).
    Link {
        id: u64,
        remote: Option<String>,
        local: Option<PathBuf>,
        expires: Option<String>,
    },
    RemoveLink {
        id: u64,
        remote: String,
    },
}

impl Job {
    fn quick(&self) -> bool {
        matches!(
            self,
            Job::Check
                | Job::List(_)
                | Job::Usage
                | Job::Settings(_)
                | Job::Link { local: None, .. }
                | Job::RemoveLink { .. }
        )
    }
}

#[derive(Debug, Clone)]
pub enum Ev {
    /// Signed in (true), signed out (false), or the CLI cannot run.
    Signed(Result<bool, String>),
    LoginUrl(String),
    Listed {
        path: String,
        result: Result<Vec<Node>, String>,
    },
    Usage(u64),
    Start {
        id: u64,
    },
    Progress {
        id: u64,
        done: u64,
        total: u64,
        files: u64,
        of: u64,
        now: String,
    },
    Done {
        id: u64,
        result: Result<String, String>,
        url: Option<String>,
    },
}

pub struct Shared {
    pub state: Mutex<State>,
    pub cancel: AtomicBool,
}

pub struct Worker {
    runner: Runner,
    cfg: Config,
    shared: Arc<Shared>,
    rx: Receiver<Job>,
    tx: Sender<Ev>,
    wake: Box<dyn Fn() + Send>,
    later: VecDeque<Job>,
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub fn human(n: u64) -> String {
    const U: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut f = n as f64;
    let mut i = 0;
    while f >= 1000.0 && i < U.len() - 1 {
        f /= 1000.0;
        i += 1;
    }
    if i == 0 {
        format!("{n} B")
    } else if f < 10.0 {
        format!("{f:.1} {}", U[i])
    } else {
        format!("{f:.0} {}", U[i])
    }
}

impl Worker {
    pub fn spawn(
        runner: Runner,
        cfg: Config,
        shared: Arc<Shared>,
        rx: Receiver<Job>,
        tx: Sender<Ev>,
        wake: Box<dyn Fn() + Send>,
    ) {
        let mut w = Worker {
            runner,
            cfg,
            shared,
            rx,
            tx,
            wake,
            later: VecDeque::new(),
        };
        std::thread::Builder::new()
            .name("cli".into())
            .spawn(move || w.run())
            .ok();
    }

    fn send(&self, e: Ev) {
        let _ = self.tx.send(e);
        (self.wake)();
    }

    fn run(&mut self) {
        loop {
            let job = match self.later.pop_front() {
                Some(j) => j,
                None => match self.rx.recv() {
                    Ok(j) => j,
                    Err(_) => return,
                },
            };
            self.shared.cancel.store(false, Ordering::SeqCst);
            self.job(job);
        }
    }

    /// Between two uploads: answer quick jobs now, keep the others.
    fn interleave(&mut self) {
        while let Ok(j) = self.rx.try_recv() {
            if j.quick() {
                self.job(j);
            } else {
                self.later.push_back(j);
            }
        }
    }

    fn cancelled(&self) -> bool {
        self.shared.cancel.load(Ordering::SeqCst)
    }

    fn signed_out(&self, out: &cli::Output) -> bool {
        if out.signed_out() {
            self.send(Ev::Signed(Ok(false)));
            true
        } else {
            false
        }
    }

    fn job(&mut self, job: Job) {
        match job {
            Job::Settings(c) => {
                self.runner.credentials = c.credentials.clone();
                self.runner.base = c.remote_base();
                self.cfg = c;
            }
            Job::Check => {
                let out = self.runner.run(&Cmd::List("/".into()));
                let r = if out.ok() {
                    Ok(true)
                } else if out.signed_out() {
                    Ok(false)
                } else {
                    Err(out.error())
                };
                self.send(Ev::Signed(r));
            }
            Job::Login => {
                let tx = self.tx.clone();
                let out = {
                    let wake = &self.wake;
                    self.runner.login(|u| {
                        let _ = tx.send(Ev::LoginUrl(u));
                        wake();
                    })
                };
                if out.ok() {
                    self.send(Ev::Signed(Ok(true)));
                } else {
                    let e = out.error();
                    self.send(Ev::Signed(if out.signed_out() || self.cancelled() {
                        Ok(false)
                    } else {
                        Err(e)
                    }));
                }
            }
            Job::Logout => {
                let _ = self.runner.run(&Cmd::Logout);
                self.send(Ev::Signed(Ok(false)));
            }
            Job::List(path) => {
                let out = self.runner.run(&Cmd::List(path.clone()));
                let result = if out.ok() {
                    Ok(cli::parse_list(&out))
                } else {
                    self.signed_out(&out);
                    Err(out.error())
                };
                self.send(Ev::Listed { path, result });
            }
            Job::Usage => {
                let out = self.runner.run(&Cmd::Size(self.cfg.remote_host()));
                if let Some(n) = out.json().and_then(|v| v["size"].as_u64()) {
                    self.send(Ev::Usage(n));
                }
            }
            Job::Backup { id } => self.backup(id),
            Job::Upload {
                id,
                locals,
                parent,
                same,
            } => {
                self.send(Ev::Start { id });
                let total: u64 = locals.iter().map(|p| du(p)).sum();
                self.send(Ev::Progress {
                    id,
                    done: 0,
                    total,
                    files: 0,
                    of: locals.len() as u64,
                    now: name_of(&locals[0]),
                });
                let r = self.ensure(&parent).and_then(|_| {
                    let out = self.runner.run(&Cmd::Upload {
                        locals: locals.clone(),
                        parent: parent.clone(),
                        same,
                    });
                    self.transfer_result(&out, "sent")
                });
                self.send(Ev::Done {
                    id,
                    result: r,
                    url: None,
                });
            }
            Job::Download { id, remotes, to } => {
                self.send(Ev::Start { id });
                let _ = std::fs::create_dir_all(&to);
                let out = self.runner.run(&Cmd::Download {
                    remotes: remotes.clone(),
                    to: to.clone(),
                });
                let r = self.transfer_result(&out, "received");
                self.send(Ev::Done {
                    id,
                    result: r.map(|m| format!("{m} to {}", crate::config::tilde(&to))),
                    url: None,
                });
            }
            Job::Link {
                id,
                remote,
                local,
                expires,
            } => {
                self.send(Ev::Start { id });
                let r = self.link(remote, local, expires);
                match r {
                    Ok(url) => self.send(Ev::Done {
                        id,
                        result: Ok("link copied".into()),
                        url: Some(url),
                    }),
                    Err(e) => self.send(Ev::Done {
                        id,
                        result: Err(e),
                        url: None,
                    }),
                }
            }
            Job::RemoveLink { id, remote } => {
                self.send(Ev::Start { id });
                let out = self.runner.run(&Cmd::RemoveUrl(remote));
                let r = if out.ok() {
                    Ok("link removed".into())
                } else {
                    self.signed_out(&out);
                    Err(out.error())
                };
                self.send(Ev::Done {
                    id,
                    result: r,
                    url: None,
                });
            }
        }
    }

    fn transfer_result(&self, out: &cli::Output, verb: &str) -> Result<String, String> {
        let sum = cli::parse_summary(out);
        if out.ok() {
            let s = sum.unwrap_or_default();
            let mut m = format!("{} {verb}", plural(s.items, "item"));
            if s.bytes > 0 {
                m.push_str(&format!(", {}", human(s.bytes)));
            }
            if s.skipped > 0 {
                m.push_str(&format!(", {} already there", s.skipped));
            }
            return Ok(m);
        }
        if self.signed_out(out) {
            return Err("signed out".into());
        }
        match sum {
            Some(s) if !s.failed.is_empty() => Err(format!(
                "{} failed: {}",
                plural(s.failed.len() as u64, "item"),
                s.failed
                    .iter()
                    .take(3)
                    .map(|(n, e)| format!("{n} ({e})"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
            _ if self.cancelled() => Err("cancelled".into()),
            _ => Err(out.error()),
        }
    }

    /// The folder exists in the Drive (made, with its parents, when not).
    fn ensure(&mut self, path: &str) -> Result<(), String> {
        let known = self
            .shared
            .state
            .lock()
            .map(|s| s.folders.contains(path))
            .unwrap_or(false);
        if known || path == "/my-files" {
            return Ok(());
        }
        let out = self.runner.run(&Cmd::Info(path.into()));
        if out.ok() {
            self.remember_folder(path);
            return Ok(());
        }
        if self.signed_out(&out) {
            return Err("signed out".into());
        }
        let Some((parent, name)) = split(path) else {
            return Err(format!("no folder {path}"));
        };
        self.ensure(&parent)?;
        let out = self.runner.run(&Cmd::MakeFolder {
            parent: parent.clone(),
            name: name.clone(),
        });
        if out.ok() || out.error().to_lowercase().contains("exist") {
            self.remember_folder(path);
            Ok(())
        } else {
            Err(format!("cannot make {path}: {}", out.error()))
        }
    }

    fn remember_folder(&self, path: &str) {
        if let Ok(mut s) = self.shared.state.lock() {
            s.folders.insert(path.to_string());
        }
    }

    fn link(
        &mut self,
        remote: Option<String>,
        local: Option<PathBuf>,
        expires: Option<String>,
    ) -> Result<String, String> {
        let remote = match (remote, local) {
            (Some(r), _) => r,
            (None, Some(l)) => {
                // In a backed-up folder and sent as it is: link that copy.
                let sent = self.cfg.remote_for(&l).filter(|_| {
                    backup::mark_of(&self.cfg, &self.shared.state.lock().unwrap(), &l)
                        == backup::Mark::Done
                });
                match sent {
                    Some(r) if l.is_file() => r,
                    _ => {
                        let up = self.cfg.remote_uploads();
                        self.ensure(&up)?;
                        let out = self.runner.run(&Cmd::Upload {
                            locals: vec![l.clone()],
                            parent: up.clone(),
                            same: Same::Revision,
                        });
                        self.transfer_result(&out, "sent")?;
                        format!("{up}/{}", cli::escape(&name_of(&l)))
                    }
                }
            }
            (None, None) => return Err("nothing to link".into()),
        };
        let out = self.runner.run(&Cmd::SetUrl {
            path: remote,
            expires,
        });
        if !out.ok() {
            self.signed_out(&out);
            return Err(out.error());
        }
        cli::parse_url(&out).ok_or_else(|| "the CLI gave no link".to_string())
    }

    fn backup(&mut self, id: u64) {
        self.send(Ev::Start { id });
        let cfg = self.cfg.clone();
        let mut skips = Vec::new();
        let mut trees = Vec::new();
        let mut missing = Vec::new();
        for f in &cfg.folders {
            if f.is_dir() {
                trees.push(backup::walk(f, &cfg, &mut skips));
            } else {
                missing.push(crate::config::tilde(f));
            }
        }
        let ops = {
            let st = self.shared.state.lock().unwrap();
            for t in &mut trees {
                backup::mark(t, &st);
            }
            backup::plan(&cfg, &st, &trees)
        };
        drop(trees);
        let total: u64 = ops.iter().map(Op::bytes).sum();
        let of: u64 = ops.iter().map(Op::count).sum();
        let (mut done, mut files) = (0u64, 0u64);
        let mut error: Option<String> = None;
        let mut saved = Instant::now();
        for op in &ops {
            self.interleave();
            if self.cancelled() {
                error = Some("cancelled".into());
                break;
            }
            let now = match op {
                Op::Folder { name, .. } => name.clone(),
                Op::Tree { local, .. } => name_of(local),
                Op::Files { files, .. } => files.first().map(|f| name_of(&f.0)).unwrap_or_default(),
            };
            self.send(Ev::Progress {
                id,
                done,
                total,
                files,
                of,
                now,
            });
            let r = self.backup_op(op);
            match r {
                Ok(failed) => {
                    if let Ok(mut st) = self.shared.state.lock() {
                        backup::record(&mut st, op, &failed);
                    }
                    if !failed.is_empty() && error.is_none() {
                        error = Some(format!(
                            "{} not sent: {}",
                            plural(failed.len() as u64, "file"),
                            failed.join(", ")
                        ));
                    }
                }
                Err(e) => {
                    error = Some(e);
                    break;
                }
            }
            done += op.bytes();
            files += op.count();
            if saved.elapsed() > Duration::from_secs(20) {
                if let Ok(st) = self.shared.state.lock() {
                    let _ = st.save();
                }
                saved = Instant::now();
            }
        }
        if error.is_none() && !missing.is_empty() {
            error = Some(format!("not found: {}", missing.join(", ")));
        }
        let result = {
            let mut st = self.shared.state.lock().unwrap();
            st.last_run = now();
            st.last_files = files;
            st.last_bytes = done;
            match &error {
                None => {
                    st.last_ok = st.last_run;
                    st.last_error.clear();
                }
                Some(e) => st.last_error = e.clone(),
            }
            let _ = st.save();
            match &error {
                None if of == 0 => Ok("up to date".to_string()),
                None => Ok(format!("{} sent, {}", plural(files, "file"), human(done))),
                Some(e) => Err(e.clone()),
            }
        };
        let result = result.map(|m| {
            if skips.is_empty() {
                m
            } else {
                format!("{m} ({} left out)", skips.len())
            }
        });
        self.send(Ev::Done {
            id,
            result,
            url: None,
        });
    }

    /// One step of a backup; Ok holds the names that failed to upload.
    fn backup_op(&mut self, op: &Op) -> Result<Vec<String>, String> {
        let (locals, parent) = match op {
            Op::Folder { parent, name } => {
                return self
                    .ensure(&format!("{parent}/{}", cli::escape(name)))
                    .map(|_| Vec::new())
            }
            Op::Tree { local, parent, .. } => (vec![local.clone()], parent.clone()),
            Op::Files { files, parent, .. } => {
                (files.iter().map(|f| f.0.clone()).collect(), parent.clone())
            }
        };
        // Files may have gone since the walk.
        let locals: Vec<PathBuf> = locals.into_iter().filter(|p| p.exists()).collect();
        if locals.is_empty() {
            return Ok(Vec::new());
        }
        self.ensure(&parent)?;
        let out = self.runner.run(&Cmd::Upload {
            locals,
            parent,
            same: Same::Revision,
        });
        if out.ok() {
            return Ok(Vec::new());
        }
        if self.signed_out(&out) {
            return Err("signed out".into());
        }
        if self.cancelled() {
            return Err("cancelled".into());
        }
        match cli::parse_summary(&out) {
            Some(s) if !s.failed.is_empty() => Ok(s.failed.into_iter().map(|f| f.0).collect()),
            _ => Err(out.error()),
        }
    }
}

fn split(path: &str) -> Option<(String, String)> {
    // The last `/` that is not escaped.
    let b = path.as_bytes();
    let mut i = b.len();
    while i > 0 {
        i -= 1;
        if b[i] == b'/' && (i == 0 || b[i - 1] != b'\\') {
            let name = path[i + 1..].replace("\\/", "/");
            let parent = &path[..i];
            if parent.is_empty() || name.is_empty() {
                return None;
            }
            return Some((parent.to_string(), name));
        }
    }
    None
}

pub fn name_of(p: &std::path::Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.display().to_string())
}

pub fn plural(n: u64, word: &str) -> String {
    if n == 1 {
        format!("1 {word}")
    } else {
        format!("{n} {word}s")
    }
}

/// The size of a file, or of everything in a folder.
pub fn du(p: &std::path::Path) -> u64 {
    let Ok(m) = std::fs::symlink_metadata(p) else {
        return 0;
    };
    if m.is_dir() {
        std::fs::read_dir(p)
            .map(|rd| rd.filter_map(Result::ok).map(|e| du(&e.path())).sum())
            .unwrap_or(0)
    } else {
        m.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drive_paths_split_into_parent_and_name() {
        assert_eq!(
            split("/my-files/NIRUX/othala"),
            Some(("/my-files/NIRUX".into(), "othala".into()))
        );
        assert_eq!(
            split("/my-files/a\\/b"),
            Some(("/my-files".into(), "a/b".into()))
        );
        assert_eq!(split("/my-files"), None);
        assert_eq!(human(999), "999 B");
        assert_eq!(human(1_500_000), "1.5 MB");
        assert_eq!(human(25_000_000_000), "25 GB");
    }
}
