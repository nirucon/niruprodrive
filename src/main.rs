//! niruprodrive: Proton Drive for NIRUX. A backup of chosen folders, files
//! sent, fetched and shared, a tray icon and a small window, through
//! Proton's own Proton Drive CLI.
mod app;
mod backup;
mod canvas;
mod cli;
mod config;
mod icons;
mod logo;
mod text;
mod theme;
mod worker;
mod x;

use serde_json::json;
use std::process::ExitCode;

const VERSION: &str = env!("CARGO_PKG_VERSION");

const HELP: &str = "\
niruprodrive                  open the window (starts the service when needed)
niruprodrive service          the tray icon and the backups, no window
                              (also: autostart, daemon)
niruprodrive upload PATH...   send files or folders to My files/NIRUX/Uploads
niruprodrive link PATH [DAYS] copy a public link to a file (sent first when
                              needed); DAYS makes it expire
niruprodrive backup           back up now
niruprodrive status           what it is doing
niruprodrive quit             stop the service
niruprodrive --version

niruprodrive is a third-party application, not made or supported by Proton.
It works through Proton's Proton Drive CLI.
Settings: ~/.config/niruprodrive/config.toml";

fn absolute(p: &str) -> String {
    let path = std::path::Path::new(p);
    if path.is_absolute() {
        p.to_string()
    } else {
        std::env::current_dir()
            .map(|d| d.join(path).display().to_string())
            .unwrap_or_else(|_| p.to_string())
    }
}

/// Bind the socket; None when another niruprodrive already answers.
fn bind() -> Result<Option<std::os::unix::net::UnixListener>, String> {
    let path = app::socket_path();
    if let Some(d) = path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    if std::os::unix::net::UnixStream::connect(&path).is_ok() {
        return Ok(None);
    }
    let _ = std::fs::remove_file(&path);
    let l = std::os::unix::net::UnixListener::bind(&path)
        .map_err(|e| format!("cannot listen on {}: {e}", path.display()))?;
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    Ok(Some(l))
}

fn serve(show: bool) -> Result<(), String> {
    // A long-running service should not hold on to the folder it was
    // started in (an installer's temporary folder, a USB stick).
    let _ = std::env::set_current_dir(theme::home());
    match bind()? {
        None => {
            if show {
                app::ask(&json!({"op": "show"}))?;
            }
            Ok(())
        }
        Some(l) => {
            let mut a = app::App::new(l, show)?;
            a.run();
            Ok(())
        }
    }
}

fn ask_print(v: serde_json::Value) -> Result<(), String> {
    let r = app::ask(&v)?;
    if r["ok"].as_bool() == Some(true) {
        Ok(())
    } else {
        Err(r["error"].as_str().unwrap_or("failed").to_string())
    }
}

fn main() -> ExitCode {
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    let r = match args.first().map(String::as_str) {
        None | Some("show" | "open") => serve(true),
        Some("service" | "autostart" | "daemon") => serve(false),
        Some("-h" | "--help" | "help") => {
            println!("{HELP}");
            Ok(())
        }
        Some("-V" | "--version" | "version") => {
            println!("niruprodrive {VERSION}");
            Ok(())
        }
        Some("upload") if args.len() > 1 => ask_print(json!({
            "op": "upload",
            "paths": args[1..].iter().map(|p| absolute(p)).collect::<Vec<_>>(),
        })),
        Some("link") if args.len() > 1 => ask_print(json!({
            "op": "link",
            "paths": [absolute(&args[1])],
            "days": args.get(2).and_then(|d| d.parse::<i64>().ok()).unwrap_or(0),
        })),
        Some("backup") => ask_print(json!({"op": "backup"})),
        Some("quit") => ask_print(json!({"op": "quit"})),
        Some("status") => app::ask(&json!({"op": "state"})).map(|v| {
            println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
        }),
        Some(other) => Err(format!("unknown command {other}; niruprodrive --help")),
    };
    match r {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("niruprodrive: {e}");
            ExitCode::FAILURE
        }
    }
}
