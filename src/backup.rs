//! The backup: a walk of the chosen folders, compared with what was sent
//! before, becomes a short list of uploads. Nothing is ever taken away in
//! the Drive: a file deleted here stays there, and a changed file becomes a
//! new revision (the Drive keeps the old ones).
//!
//! Only what changed is sent. A folder that is entirely new and clean (no
//! excluded names inside) goes in one upload; otherwise changed files go in
//! batches per folder. The Proton CLI starts once per upload, so few, large
//! uploads are fast, and the Drive is never walked from here.
use crate::cli::escape;
use crate::config::Config;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// A folder upload is at most this big, so progress moves now and then.
const TREE_BYTES: u64 = 512 << 20;
const TREE_FILES: u64 = 5000;
/// One batch of files.
const BATCH_FILES: usize = 200;
const BATCH_BYTES: u64 = 256 << 20;

/// Size and modification time (ns): a file is unchanged while both are.
pub type Stamp = (u64, i64);

#[derive(Debug, Default, Clone, PartialEq)]
pub struct State {
    pub files: HashMap<PathBuf, Stamp>,
    /// Folders in the Drive known to exist.
    pub folders: HashSet<String>,
    pub last_run: i64,
    pub last_ok: i64,
    pub last_error: String,
    /// Files and bytes sent in the last run.
    pub last_files: u64,
    pub last_bytes: u64,
}

pub fn state_path() -> PathBuf {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| crate::theme::home().join(".local/state"));
    base.join("niruprodrive/backup.json")
}

impl State {
    pub fn load() -> Self {
        let Ok(text) = std::fs::read_to_string(state_path()) else {
            return Self::default();
        };
        let Ok(v) = serde_json::from_str::<Value>(&text) else {
            return Self::default();
        };
        let mut s = Self::default();
        if let Some(o) = v["files"].as_object() {
            for (k, x) in o {
                if let (Some(size), Some(t)) = (x[0].as_u64(), x[1].as_i64()) {
                    s.files.insert(PathBuf::from(k), (size, t));
                }
            }
        }
        if let Some(a) = v["folders"].as_array() {
            s.folders = a
                .iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect();
        }
        s.last_run = v["last_run"].as_i64().unwrap_or(0);
        s.last_ok = v["last_ok"].as_i64().unwrap_or(0);
        s.last_error = v["last_error"].as_str().unwrap_or("").to_string();
        s.last_files = v["last_files"].as_u64().unwrap_or(0);
        s.last_bytes = v["last_bytes"].as_u64().unwrap_or(0);
        s
    }

    pub fn save(&self) -> std::io::Result<()> {
        let p = state_path();
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d)?;
        }
        let files: serde_json::Map<String, Value> = self
            .files
            .iter()
            .map(|(k, (s, t))| (k.to_string_lossy().into_owned(), json!([s, t])))
            .collect();
        let mut folders: Vec<&String> = self.folders.iter().collect();
        folders.sort();
        let v = json!({
            "files": files,
            "folders": folders,
            "last_run": self.last_run,
            "last_ok": self.last_ok,
            "last_error": self.last_error,
            "last_files": self.last_files,
            "last_bytes": self.last_bytes,
        });
        let tmp = p.with_extension("json.new");
        std::fs::write(&tmp, v.to_string())?;
        std::fs::rename(tmp, p)
    }

    pub fn sent(&self, path: &Path, stamp: Stamp) -> bool {
        self.files.get(path) == Some(&stamp)
    }
}

pub fn stamp(m: &std::fs::Metadata) -> Stamp {
    use std::os::unix::fs::MetadataExt;
    (m.len(), m.mtime() * 1_000_000_000 + m.mtime_nsec())
}

/// One folder as found on disk, below a backup folder.
#[derive(Debug, Default)]
pub struct Tree {
    pub path: PathBuf,
    pub files: Vec<(PathBuf, Stamp)>,
    pub dirs: Vec<Tree>,
    /// Nothing below was left out (excluded, too big or unreadable).
    pub clean: bool,
    pub bytes: u64,
    pub count: u64,
    /// Files below that changed since the last upload, and that were sent.
    pub changed: u64,
    pub known: u64,
}

/// Something left out of the backup, and why.
#[derive(Debug, Clone, PartialEq)]
pub struct Skip {
    pub path: PathBuf,
    pub why: &'static str,
}

fn name_ok(name: &str) -> bool {
    !name.contains('\\') && !name.chars().any(|c| c.is_control())
}

pub fn walk(path: &Path, cfg: &Config, skips: &mut Vec<Skip>) -> Tree {
    let mut t = Tree {
        path: path.to_path_buf(),
        clean: true,
        ..Default::default()
    };
    let rd = match std::fs::read_dir(path) {
        Ok(r) => r,
        Err(_) => {
            skips.push(Skip {
                path: path.to_path_buf(),
                why: "cannot be read",
            });
            t.clean = false;
            return t;
        }
    };
    let max = cfg.max_file_mb.saturating_mul(1 << 20);
    let mut entries: Vec<_> = rd.filter_map(Result::ok).collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let name = e.file_name();
        let Some(name) = name.to_str() else {
            skips.push(Skip {
                path: e.path(),
                why: "the name is not UTF-8",
            });
            t.clean = false;
            continue;
        };
        if cfg.excluded(name) {
            t.clean = false;
            continue;
        }
        if !name_ok(name) {
            skips.push(Skip {
                path: e.path(),
                why: "the name has a backslash or control character",
            });
            t.clean = false;
            continue;
        }
        let Ok(m) = std::fs::symlink_metadata(e.path()) else {
            t.clean = false;
            continue;
        };
        let ft = m.file_type();
        if ft.is_dir() {
            let sub = walk(&e.path(), cfg, skips);
            t.clean &= sub.clean;
            t.bytes += sub.bytes;
            t.count += sub.count;
            t.dirs.push(sub);
        } else if ft.is_file() {
            if m.len() > max {
                skips.push(Skip {
                    path: e.path(),
                    why: "larger than max_file_mb",
                });
                t.clean = false;
                continue;
            }
            t.bytes += m.len();
            t.count += 1;
            t.files.push((e.path(), stamp(&m)));
        } else {
            // Links, sockets and devices are not files to keep. A link
            // inside a folder that is sent whole would be followed by the
            // CLI, so it makes the folder go file by file.
            t.clean = false;
        }
    }
    t
}

/// Count what changed and what was sent, below each folder.
pub fn mark(t: &mut Tree, state: &State) {
    let mut changed = 0;
    let mut known = 0;
    for (p, s) in &t.files {
        match state.files.get(p) {
            Some(x) if x == s => known += 1,
            Some(_) => {
                changed += 1;
                known += 1;
            }
            None => changed += 1,
        }
    }
    for d in &mut t.dirs {
        mark(d, state);
        changed += d.changed;
        known += d.known;
    }
    t.changed = changed;
    t.known = known;
}

#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    /// Make sure the folder `parent/name` exists in the Drive.
    Folder { parent: String, name: String },
    /// Upload a new folder whole into `parent`.
    Tree {
        local: PathBuf,
        parent: String,
        files: Vec<(PathBuf, Stamp)>,
        folders: Vec<String>,
        bytes: u64,
    },
    /// Upload these files into `parent`.
    Files {
        files: Vec<(PathBuf, Stamp)>,
        parent: String,
        bytes: u64,
    },
}

impl Op {
    pub fn bytes(&self) -> u64 {
        match self {
            Op::Folder { .. } => 0,
            Op::Tree { bytes, .. } | Op::Files { bytes, .. } => *bytes,
        }
    }

    pub fn count(&self) -> u64 {
        match self {
            Op::Folder { .. } => 0,
            Op::Tree { files, .. } | Op::Files { files, .. } => files.len() as u64,
        }
    }
}

fn all_files(t: &Tree, out: &mut Vec<(PathBuf, Stamp)>, folders: &mut Vec<String>, remote: &str) {
    folders.push(remote.to_string());
    out.extend(t.files.iter().cloned());
    for d in &t.dirs {
        let name = d.path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        all_files(d, out, folders, &format!("{remote}/{}", escape(name)));
    }
}

fn plan_dir(t: &Tree, remote: &str, state: &State, ops: &mut Vec<Op>) {
    // Changed files, in batches.
    let mut batch: Vec<(PathBuf, Stamp)> = Vec::new();
    let mut bytes = 0;
    for (p, s) in &t.files {
        if state.sent(p, *s) {
            continue;
        }
        if !batch.is_empty() && (batch.len() >= BATCH_FILES || bytes + s.0 > BATCH_BYTES) {
            ops.push(Op::Files {
                files: std::mem::take(&mut batch),
                parent: remote.into(),
                bytes,
            });
            bytes = 0;
        }
        bytes += s.0;
        batch.push((p.clone(), *s));
    }
    if !batch.is_empty() {
        ops.push(Op::Files {
            files: batch,
            parent: remote.into(),
            bytes,
        });
    }
    for d in &t.dirs {
        if d.changed == 0 {
            continue;
        }
        let name = d.path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let sub = format!("{remote}/{}", escape(name));
        if d.known == 0 && d.clean && d.bytes <= TREE_BYTES && d.count <= TREE_FILES {
            let mut files = Vec::new();
            let mut folders = Vec::new();
            all_files(d, &mut files, &mut folders, &sub);
            ops.push(Op::Tree {
                local: d.path.clone(),
                parent: remote.into(),
                files,
                folders,
                bytes: d.bytes,
            });
            continue;
        }
        if !state.folders.contains(&sub) {
            ops.push(Op::Folder {
                parent: remote.into(),
                name: name.into(),
            });
        }
        plan_dir(d, &sub, state, ops);
    }
}

/// The uploads that bring the Drive up to date with these folders.
pub fn plan(cfg: &Config, state: &State, trees: &[Tree]) -> Vec<Op> {
    let mut ops = Vec::new();
    let mut made: HashSet<String> = HashSet::new();
    for t in trees {
        if t.changed == 0 {
            continue;
        }
        let Some(remote) = cfg.remote_for(&t.path) else {
            continue;
        };
        // The folders down to it: /my-files/NIRUX/othala/Documents …
        let mut parent = String::from("/my-files");
        for part in remote.trim_start_matches("/my-files/").split('/') {
            let here = format!("{parent}/{part}");
            if !state.folders.contains(&here) && made.insert(here.clone()) {
                ops.push(Op::Folder {
                    parent: parent.clone(),
                    name: part.into(),
                });
            }
            parent = here;
        }
        plan_dir(t, &remote, state, &mut ops);
    }
    ops
}

/// The state after an upload succeeded (`failed` names were not sent).
pub fn record(state: &mut State, op: &Op, failed: &[String]) {
    let gone = |p: &Path| {
        p.file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| failed.iter().any(|f| f == n))
    };
    match op {
        Op::Folder { parent, name } => {
            state.folders.insert(format!("{parent}/{}", escape(name)));
        }
        Op::Tree { files, folders, .. } => {
            for (p, s) in files {
                if !gone(p) {
                    state.files.insert(p.clone(), *s);
                }
            }
            if failed.is_empty() {
                state.folders.extend(folders.iter().cloned());
            }
        }
        Op::Files { files, .. } => {
            for (p, s) in files {
                if !gone(p) {
                    state.files.insert(p.clone(), *s);
                }
            }
        }
    }
}

/// How a file stands, for the badges in NIRUFM.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    /// Not in a backed-up folder (or left out).
    None,
    /// In the Drive as it is now.
    Done,
    /// Changed or new since the last backup.
    Pending,
}

impl Mark {
    pub fn name(self) -> &'static str {
        match self {
            Mark::None => "none",
            Mark::Done => "done",
            Mark::Pending => "pending",
        }
    }
}

pub fn mark_of(cfg: &Config, state: &State, path: &Path) -> Mark {
    let Some(root) = cfg.folders.iter().find(|r| path.starts_with(r)) else {
        return Mark::None;
    };
    let below = path.strip_prefix(root).unwrap_or(path);
    if below
        .components()
        .any(|c| c.as_os_str().to_str().is_none_or(|n| cfg.excluded(n)))
    {
        return Mark::None;
    }
    let Ok(m) = std::fs::symlink_metadata(path) else {
        return Mark::None;
    };
    if m.is_dir() {
        return if state
            .folders
            .contains(&cfg.remote_for(path).unwrap_or_default())
        {
            Mark::Done
        } else {
            Mark::Pending
        };
    }
    if !m.is_file() || m.len() > cfg.max_file_mb.saturating_mul(1 << 20) {
        return Mark::None;
    }
    if state.sent(path, stamp(&m)) {
        Mark::Done
    } else {
        Mark::Pending
    }
}

/// Running on battery: a battery present and no mains supply online.
pub fn on_battery() -> bool {
    let Ok(rd) = std::fs::read_dir("/sys/class/power_supply") else {
        return false;
    };
    let mut battery = false;
    for e in rd.filter_map(Result::ok) {
        let p = e.path();
        let kind = std::fs::read_to_string(p.join("type")).unwrap_or_default();
        match kind.trim() {
            "Mains" | "USB" => {
                if std::fs::read_to_string(p.join("online")).is_ok_and(|s| s.trim() == "1") {
                    return false;
                }
            }
            "Battery"
                if std::fs::read_to_string(p.join("scope"))
                    .map_or(true, |s| s.trim() != "Device") =>
            {
                battery = true;
            }
            _ => {}
        }
    }
    battery
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sandbox(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("niruprodrive-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn cfg_for(root: &Path) -> Config {
        Config {
            host: "test".into(),
            folders: vec![root.to_path_buf()],
            ..Default::default()
        }
    }

    fn run(cfg: &Config, state: &State) -> (Vec<Op>, Vec<Skip>) {
        let mut skips = Vec::new();
        let mut trees: Vec<Tree> = cfg
            .folders
            .iter()
            .map(|f| walk(f, cfg, &mut skips))
            .collect();
        for t in &mut trees {
            mark(t, state);
        }
        (plan(cfg, state, &trees), skips)
    }

    #[test]
    fn a_first_backup_sends_new_folders_whole_and_then_only_changes() {
        let root = sandbox("plan").join("Documents");
        std::fs::create_dir_all(root.join("work/deep")).unwrap();
        std::fs::create_dir_all(root.join("code/target")).unwrap();
        std::fs::write(root.join("top.txt"), "top").unwrap();
        std::fs::write(root.join("work/a.txt"), "a").unwrap();
        std::fs::write(root.join("work/deep/b.txt"), "bb").unwrap();
        std::fs::write(root.join("code/main.rs"), "fn main(){}").unwrap();
        std::fs::write(root.join("code/target/big.o"), "x").unwrap();
        let cfg = cfg_for(&root);
        let mut state = State::default();
        let (ops, _) = run(&cfg, &state);
        let base = cfg.remote_for(&root).unwrap();
        // The chain of folders down to the backup folder comes first.
        let folders: Vec<String> = ops
            .iter()
            .filter_map(|o| match o {
                Op::Folder { parent, name } => Some(format!("{parent}/{name}")),
                _ => None,
            })
            .collect();
        assert!(folders.contains(&"/my-files/NIRUX".to_string()));
        assert!(folders.contains(&"/my-files/NIRUX/test".to_string()));
        assert!(folders.contains(&base));
        // work/ is new and clean: one upload of the whole folder.
        assert!(ops.iter().any(|o| matches!(o, Op::Tree { local, files, .. }
            if local.ends_with("work") && files.len() == 2)));
        // code/ holds an excluded target/: it goes file by file instead.
        assert!(ops
            .iter()
            .any(|o| matches!(o, Op::Files { files, parent, .. }
            if parent == &format!("{base}/code") && files.len() == 1)));
        assert!(!ops.iter().any(|o| match o {
            Op::Files { files, .. } | Op::Tree { files, .. } =>
                files.iter().any(|(p, _)| p.ends_with("big.o")),
            _ => false,
        }));
        for o in &ops {
            record(&mut state, o, &[]);
        }
        // Nothing changed: nothing to do.
        assert_eq!(run(&cfg, &state).0, vec![]);
        // One file changes: only it goes, into the folder it is in.
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(root.join("work/deep/b.txt"), "changed").unwrap();
        let (ops, _) = run(&cfg, &state);
        assert_eq!(ops.len(), 1, "{ops:?}");
        assert!(matches!(&ops[0], Op::Files { files, parent, .. }
            if files.len() == 1 && parent == &format!("{base}/work/deep")));
        // A deleted file is simply not there: nothing is removed.
        std::fs::remove_file(root.join("top.txt")).unwrap();
        record(&mut state, &ops[0], &[]);
        assert_eq!(run(&cfg, &state).0, vec![]);
        let _ = std::fs::remove_dir_all(root.parent().unwrap());
    }

    #[test]
    fn failed_files_are_sent_again_and_big_ones_are_left_out() {
        let root = sandbox("fail");
        std::fs::write(root.join("a.txt"), "a").unwrap();
        std::fs::write(root.join("b.txt"), "b").unwrap();
        std::fs::write(root.join("huge.bin"), vec![0u8; 2 << 20]).unwrap();
        let mut cfg = cfg_for(&root);
        cfg.max_file_mb = 1;
        let mut state = State::default();
        let (ops, skips) = run(&cfg, &state);
        assert_eq!(skips.len(), 1);
        assert!(skips[0].path.ends_with("huge.bin"));
        for o in &ops {
            record(&mut state, o, &["b.txt".to_string()]);
        }
        let (ops, _) = run(&cfg, &state);
        assert!(matches!(&ops[..], [Op::Files { files, .. }]
            if files.len() == 1 && files[0].0.ends_with("b.txt")));
        assert_eq!(mark_of(&cfg, &state, &root.join("a.txt")), Mark::Done);
        assert_eq!(mark_of(&cfg, &state, &root.join("b.txt")), Mark::Pending);
        assert_eq!(mark_of(&cfg, &state, &root.join("huge.bin")), Mark::None);
        assert_eq!(
            mark_of(&cfg, &state, Path::new("/etc/hostname")),
            Mark::None
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn many_files_go_in_batches() {
        let root = sandbox("batch");
        std::fs::create_dir_all(root.join("x")).unwrap();
        std::fs::write(root.join("x/skip.tmp"), "").unwrap();
        for i in 0..450 {
            std::fs::write(root.join(format!("x/f{i}")), "1").unwrap();
        }
        let cfg = cfg_for(&root);
        let (ops, _) = run(&cfg, &State::default());
        let sizes: Vec<u64> = ops
            .iter()
            .filter(|o| matches!(o, Op::Files { .. }))
            .map(Op::count)
            .collect();
        assert_eq!(sizes, vec![200, 200, 50]);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_state_survives_saving() {
        let d = sandbox("state");
        std::env::set_var("XDG_STATE_HOME", &d);
        let mut s = State::default();
        s.files
            .insert("/a b/ö.txt".into(), (3, 1_700_000_000_123_456_789));
        s.folders.insert("/my-files/NIRUX".into());
        s.last_ok = 5;
        s.last_error = "x".into();
        s.save().unwrap();
        assert_eq!(State::load(), s);
        std::env::remove_var("XDG_STATE_HOME");
        let _ = std::fs::remove_dir_all(d);
    }
}
