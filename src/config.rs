//! `~/.config/niruprodrive/config.toml`: what this computer backs up, how
//! often, and where in the Drive. Every computer has its own file, so each
//! one chooses its own folders.
use crate::theme::{config_home, home};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Every {
    Manual,
    Hourly,
    Daily,
}

impl Every {
    pub fn name(self) -> &'static str {
        match self {
            Every::Manual => "manual",
            Every::Hourly => "hourly",
            Every::Daily => "daily",
        }
    }

    pub fn seconds(self) -> Option<u64> {
        match self {
            Every::Manual => None,
            Every::Hourly => Some(3600),
            Every::Daily => Some(86400),
        }
    }

    pub fn next(self) -> Self {
        match self {
            Every::Manual => Every::Hourly,
            Every::Hourly => Every::Daily,
            Every::Daily => Every::Manual,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    /// The folders this computer backs up.
    pub folders: Vec<PathBuf>,
    /// Names (with `*` wildcards) that are never backed up, at any depth.
    pub exclude: Vec<String>,
    pub every: Every,
    /// Back up while the computer runs on battery.
    pub on_battery: bool,
    /// Files larger than this are left out (MB).
    pub max_file_mb: u64,
    /// The folder in "My files" that niruprodrive writes in.
    pub remote: String,
    /// This computer's folder in it (the host name).
    pub host: String,
    /// Where downloads go.
    pub downloads: PathBuf,
    /// "keychain" (the Secret Service) or "pass".
    pub credentials: String,
    /// The Proton Drive CLI, when not the one the installer put in place.
    pub cli: Option<PathBuf>,
}

pub const DEFAULT_EXCLUDE: &[&str] = &[
    ".cache",
    ".Trash-*",
    "node_modules",
    "target",
    "__pycache__",
    ".venv",
    "*.tmp",
    "*.part",
    "*.crdownload",
    "*.swp",
    "*.iso",
    "*.qcow2",
    "*.vdi",
    ".DS_Store",
];

pub fn hostname() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .or_else(|_| std::fs::read_to_string("/etc/hostname"))
        .map(|s| s.trim().to_string())
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "computer".into())
}

impl Default for Config {
    fn default() -> Self {
        Self {
            folders: Vec::new(),
            exclude: DEFAULT_EXCLUDE.iter().map(|s| s.to_string()).collect(),
            every: Every::Hourly,
            on_battery: false,
            max_file_mb: 4096,
            remote: "NIRUX".into(),
            host: hostname(),
            downloads: home().join("Downloads"),
            credentials: "keychain".into(),
            cli: None,
        }
    }
}

pub fn dir() -> PathBuf {
    config_home().join("niruprodrive")
}

pub fn path() -> PathBuf {
    dir().join("config.toml")
}

fn expand(p: &str) -> PathBuf {
    match p.strip_prefix("~/") {
        Some(rest) => home().join(rest),
        None if p == "~" => home(),
        None => PathBuf::from(p),
    }
}

/// `~/…` for paths in the home folder, the way they are written in the file.
pub fn tilde(p: &Path) -> String {
    let h = home();
    match p.strip_prefix(&h) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".into(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => p.display().to_string(),
    }
}

/// A name that is safe as one path part in the Drive.
fn clean_name(s: &str) -> String {
    let t: String = s
        .chars()
        .filter(|c| !c.is_control() && *c != '/' && *c != '\\')
        .collect();
    let t = t.trim().to_string();
    if t.is_empty() || t == "." || t == ".." {
        String::new()
    } else {
        t
    }
}

impl Config {
    pub fn parse(text: &str) -> Self {
        let mut c = Config::default();
        let Ok(v) = text.parse::<toml::Value>() else {
            return c;
        };
        let b = v.get("backup");
        let get = |k: &str| b.and_then(|t| t.get(k)).or_else(|| v.get(k));
        if let Some(a) = get("folders").and_then(|x| x.as_array()) {
            c.folders = a
                .iter()
                .filter_map(|x| x.as_str())
                .map(expand)
                .filter(|p| p.is_absolute())
                .collect();
        }
        if let Some(a) = get("exclude").and_then(|x| x.as_array()) {
            c.exclude = a
                .iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect();
        }
        if let Some(e) = get("every").and_then(|x| x.as_str()) {
            c.every = match e {
                "manual" | "never" => Every::Manual,
                "daily" => Every::Daily,
                _ => Every::Hourly,
            };
        }
        if let Some(x) = get("on_battery").and_then(|x| x.as_bool()) {
            c.on_battery = x;
        }
        if let Some(x) = get("max_file_mb").and_then(|x| x.as_integer()) {
            c.max_file_mb = x.max(1) as u64;
        }
        let drive = v.get("drive");
        let dget = |k: &str| drive.and_then(|t| t.get(k)).and_then(|x| x.as_str());
        if let Some(r) = dget("folder").map(clean_name).filter(|s| !s.is_empty()) {
            c.remote = r;
        }
        if let Some(h) = dget("computer").map(clean_name).filter(|s| !s.is_empty()) {
            c.host = h;
        }
        if let Some(d) = dget("downloads") {
            c.downloads = expand(d);
        }
        if let Some(cr) = dget("credentials") {
            if cr == "pass" || cr == "keychain" {
                c.credentials = cr.into();
            }
        }
        if let Some(p) = dget("cli") {
            c.cli = Some(expand(p));
        }
        c
    }

    pub fn load() -> Self {
        std::fs::read_to_string(path())
            .map(|t| Config::parse(&t))
            .unwrap_or_default()
    }

    pub fn to_toml(&self) -> String {
        let q = |s: &str| {
            let mut o = String::from("\"");
            for ch in s.chars() {
                match ch {
                    '"' => o.push_str("\\\""),
                    '\\' => o.push_str("\\\\"),
                    c if c.is_control() => o.push_str(&format!("\\u{:04X}", c as u32)),
                    c => o.push(c),
                }
            }
            o.push('"');
            o
        };
        let list = |v: Vec<String>| {
            if v.is_empty() {
                "[]".to_string()
            } else {
                format!(
                    "[\n{}\n]",
                    v.iter()
                        .map(|s| format!("  {},", q(s)))
                        .collect::<Vec<_>>()
                        .join("\n")
                )
            }
        };
        let mut s = String::from(
            "# niruprodrive: this computer's settings (the window changes them too).\n\n[backup]\n",
        );
        s.push_str("# Folders backed up to My files/");
        s.push_str(&format!(
            "{}/{}/ in Proton Drive.\n",
            self.remote, self.host
        ));
        s.push_str(&format!(
            "folders = {}\n",
            list(self.folders.iter().map(|p| tilde(p)).collect())
        ));
        s.push_str("# Names never backed up, at any depth; * matches anything.\n");
        s.push_str(&format!("exclude = {}\n", list(self.exclude.clone())));
        s.push_str("# manual, hourly or daily\n");
        s.push_str(&format!("every = {}\n", q(self.every.name())));
        s.push_str(&format!("on_battery = {}\n", self.on_battery));
        s.push_str(&format!("max_file_mb = {}\n\n[drive]\n", self.max_file_mb));
        s.push_str(&format!("folder = {}\n", q(&self.remote)));
        s.push_str(&format!("computer = {}\n", q(&self.host)));
        s.push_str(&format!("downloads = {}\n", q(&tilde(&self.downloads))));
        s.push_str("# keychain (the Secret Service) or pass\n");
        s.push_str(&format!("credentials = {}\n", q(&self.credentials)));
        if let Some(c) = &self.cli {
            s.push_str(&format!("cli = {}\n", q(&c.display().to_string())));
        }
        s
    }

    pub fn save(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(dir())?;
        let tmp = dir().join(".config.toml.new");
        std::fs::write(&tmp, self.to_toml())?;
        std::fs::rename(tmp, path())
    }

    /// `/my-files/NIRUX`
    pub fn remote_base(&self) -> String {
        format!("/my-files/{}", self.remote)
    }

    /// `/my-files/NIRUX/othala`
    pub fn remote_host(&self) -> String {
        format!("{}/{}", self.remote_base(), self.host)
    }

    /// Where files uploaded by hand go: `/my-files/NIRUX/Uploads`.
    pub fn remote_uploads(&self) -> String {
        format!("{}/Uploads", self.remote_base())
    }

    /// The folder in the Drive a local folder is backed up to: the path
    /// under the home folder, or under `root` for the rest of the system.
    pub fn remote_for(&self, local: &Path) -> Option<String> {
        let h = home();
        let (base, rest) = match local.strip_prefix(&h) {
            Ok(r) => (self.remote_host(), r.to_path_buf()),
            Err(_) => (
                format!("{}/root", self.remote_host()),
                local.strip_prefix("/").ok()?.to_path_buf(),
            ),
        };
        let mut out = base;
        for part in rest.components() {
            let s = part.as_os_str().to_str()?;
            if clean_name(s) != s {
                return None;
            }
            out.push('/');
            out.push_str(s);
        }
        Some(out)
    }

    pub fn excluded(&self, name: &str) -> bool {
        self.exclude.iter().any(|p| glob(p, name))
    }
}

/// `*` matches any run of characters; everything else matches itself.
pub fn glob(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let n: Vec<char> = name.chars().collect();
    let (mut pi, mut ni) = (0, 0);
    let (mut star, mut mark) = (None, 0);
    while ni < n.len() {
        if pi < p.len() && p[pi] != '*' && p[pi] == n[ni] {
            pi += 1;
            ni += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ni;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ni = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcards() {
        assert!(glob("*.iso", "debian.iso"));
        assert!(!glob("*.iso", "debian.iso.txt"));
        assert!(glob(".Trash-*", ".Trash-1000"));
        assert!(glob("target", "target"));
        assert!(!glob("target", "targets"));
        assert!(glob("a*b*c", "aXXbYYc"));
        assert!(glob("*", ""));
    }

    #[test]
    fn a_config_survives_writing_and_reading() {
        let mut c = Config {
            folders: vec![home().join("Documents"), PathBuf::from("/srv/data")],
            every: Every::Daily,
            on_battery: true,
            host: "othala".into(),
            ..Default::default()
        };
        c.exclude.push("say \"hi\"".into());
        let back = Config::parse(&c.to_toml());
        assert_eq!(back, c);
    }

    #[test]
    fn remote_paths_follow_the_home_folder() {
        let c = Config {
            host: "void".into(),
            ..Default::default()
        };
        assert_eq!(
            c.remote_for(&home().join("Documents/work")).as_deref(),
            Some("/my-files/NIRUX/void/Documents/work")
        );
        assert_eq!(
            c.remote_for(Path::new("/srv/data")).as_deref(),
            Some("/my-files/NIRUX/void/root/srv/data")
        );
        // A name the Drive path syntax cannot carry is not mapped.
        assert_eq!(c.remote_for(&home().join("a\\b")), None);
    }

    #[test]
    fn bad_names_from_the_file_are_cleaned() {
        let c = Config::parse("[drive]\nfolder = \"../x\"\ncomputer = \"a/b\"\n");
        assert_eq!(c.remote, "..x");
        assert_eq!(c.host, "ab");
        let c = Config::parse("[drive]\nfolder = \"..\"\n");
        assert_eq!(c.remote, "NIRUX");
    }
}
