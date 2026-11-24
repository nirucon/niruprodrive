//! Staying up. The service writes down what stopped it, and a small
//! supervisor starts it again when anything other than Quit, a logout or a
//! deliberate stop of both ends it (a crash, a kill, a signal sent to the
//! service alone).
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::{Duration, Instant};

/// The service's wake-up eventfd, so a signal ends its poll at once.
pub static WAKE: AtomicI32 = AtomicI32::new(-1);
/// The last stop signal and the process that sent it (0 when none).
static SIGNAL: AtomicI32 = AtomicI32::new(0);
static SENDER: AtomicI32 = AtomicI32::new(0);

/// Exit status of a service that a signal stopped: the supervisor starts it
/// again unless it was stopped itself.
pub const STOPPED_BY_SIGNAL: i32 = 3;
/// Set in the service's environment by the supervisor.
pub const CHILD_ENV: &str = "NIRUPRODRIVE_SUPERVISED";

extern "C" fn on_signal(sig: i32, info: *mut libc::siginfo_t, _: *mut libc::c_void) {
    let pid = if info.is_null() {
        0
    } else {
        unsafe { (*info).si_pid() }
    };
    SENDER.store(pid, Ordering::SeqCst);
    SIGNAL.store(sig, Ordering::SeqCst);
    let fd = WAKE.load(Ordering::SeqCst);
    if fd >= 0 {
        let v: u64 = 1;
        unsafe { libc::write(fd, &v as *const u64 as *const libc::c_void, 8) };
    }
}

/// TERM, INT and HUP are noted instead of ending the process at once, and a
/// closed reader (SIGPIPE) is an error to handle, never a silent death.
/// `restart` keeps interrupted system calls going (the service); the
/// supervisor wants them interrupted.
pub fn catch(restart: bool) {
    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = on_signal as *const () as usize;
        sa.sa_flags = libc::SA_SIGINFO | if restart { libc::SA_RESTART } else { 0 };
        libc::sigemptyset(&mut sa.sa_mask);
        for s in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] {
            libc::sigaction(s, &sa, std::ptr::null_mut());
        }
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }
}

/// The stop signal received so far, with its sender.
pub fn received() -> Option<(i32, i32)> {
    match SIGNAL.load(Ordering::SeqCst) {
        0 => None,
        s => Some((s, SENDER.load(Ordering::SeqCst))),
    }
}

fn signal_name(sig: i32) -> String {
    match sig {
        libc::SIGHUP => "HUP".into(),
        libc::SIGINT => "INT".into(),
        libc::SIGKILL => "KILL".into(),
        libc::SIGSEGV => "SEGV".into(),
        libc::SIGABRT => "ABRT".into(),
        libc::SIGBUS => "BUS".into(),
        libc::SIGPIPE => "PIPE".into(),
        libc::SIGTERM => "TERM".into(),
        n => n.to_string(),
    }
}

/// "TERM from 1234 (pkill)".
pub fn describe(sig: i32, pid: i32) -> String {
    let mut s = signal_name(sig);
    if pid > 0 {
        let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).unwrap_or_default();
        let comm = comm.trim();
        if comm.is_empty() {
            s.push_str(&format!(" from process {pid}"));
        } else {
            s.push_str(&format!(" from process {pid} ({comm})"));
        }
    } else if pid == 0 {
        s.push_str(" from the kernel or an unknown sender");
    }
    s
}

pub fn log(msg: &str) {
    let t = unsafe {
        let now = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&now, &mut tm);
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            tm.tm_year + 1900,
            tm.tm_mon + 1,
            tm.tm_mday,
            tm.tm_hour,
            tm.tm_min,
            tm.tm_sec
        )
    };
    eprintln!("{t} niruprodrive: {msg}");
}

/// What the supervisor does when the service ended.
#[derive(Debug, PartialEq)]
pub enum After {
    Stop,
    Restart,
    GiveUp,
}

/// Restarts are spaced and limited: one that fails within 10 s of its start
/// three times in a row, or a sixth within ten minutes, is given up on.
pub struct Policy {
    starts: Vec<Instant>,
    quick: u32,
}

impl Policy {
    pub fn new() -> Self {
        Policy {
            starts: Vec::new(),
            quick: 0,
        }
    }

    /// `code` is the exit status, or None when a signal killed it.
    pub fn after(&mut self, code: Option<i32>, ran: Duration, stopped: bool) -> After {
        if stopped || code == Some(0) {
            return After::Stop;
        }
        if ran < Duration::from_secs(10) {
            self.quick += 1;
        } else {
            self.quick = 0;
        }
        let now = Instant::now();
        self.starts
            .retain(|t| now.duration_since(*t) < Duration::from_secs(600));
        self.starts.push(now);
        if self.quick >= 3 || self.starts.len() > 5 {
            After::GiveUp
        } else {
            After::Restart
        }
    }
}

/// Run the service under a supervisor. `args` start the service process
/// (the same program with `service --child`, the first one maybe showing
/// its window).
pub fn supervise(show: bool) -> i32 {
    catch(false);
    let exe = std::env::current_exe().unwrap_or_else(|_| "niruprodrive".into());
    let mut policy = Policy::new();
    let mut show = show;
    loop {
        let started = Instant::now();
        let mut cmd = std::process::Command::new(&exe);
        cmd.arg(if show { "show" } else { "service" })
            .env(CHILD_ENV, "1");
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                log(&format!("cannot start the service: {e}"));
                return 1;
            }
        };
        show = false;
        let pid = child.id() as i32;
        let mut stopped = false;
        let status = loop {
            let mut st: libc::c_int = 0;
            let r = unsafe { libc::waitpid(pid, &mut st, 0) };
            if r == pid {
                break st;
            }
            if let Some((sig, _)) = received() {
                if !stopped {
                    stopped = true;
                    unsafe { libc::kill(pid, sig) };
                }
            } else if r < 0 && std::io::Error::last_os_error().raw_os_error() != Some(libc::EINTR) {
                // Lost the child (it cannot be waited for): nothing to watch.
                let _ = child.try_wait();
                return 1;
            }
        };
        let (code, what) = if libc::WIFEXITED(status) {
            let c = libc::WEXITSTATUS(status);
            (Some(c), format!("exited with status {c}"))
        } else {
            let s = libc::WTERMSIG(status);
            (None, format!("was killed by signal {}", signal_name(s)))
        };
        let stopped = stopped || received().is_some();
        // Another niruprodrive answers (started by hand meanwhile): leave it.
        let other = crate::app::ask(&serde_json::json!({"op": "ping"})).is_ok();
        match policy.after(code, started.elapsed(), stopped || other) {
            After::Stop => return code.unwrap_or(0),
            After::GiveUp => {
                log(&format!(
                    "the service {what}; it stops too often, so it is not started again"
                ));
                return 1;
            }
            After::Restart => {
                log(&format!("the service {what}; starting it again"));
                std::thread::sleep(Duration::from_secs(2));
                if received().is_some() {
                    return 0;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quit_and_a_stop_end_the_supervisor() {
        let mut p = Policy::new();
        assert_eq!(p.after(Some(0), Duration::from_secs(1), false), After::Stop);
        assert_eq!(p.after(None, Duration::from_secs(100), true), After::Stop);
    }

    #[test]
    fn a_kill_restarts_but_not_forever() {
        let mut p = Policy::new();
        assert_eq!(
            p.after(None, Duration::from_secs(100), false),
            After::Restart
        );
        assert_eq!(
            p.after(Some(STOPPED_BY_SIGNAL), Duration::from_secs(100), false),
            After::Restart
        );
        // Failing right after each start: given up on the third time.
        assert_eq!(
            p.after(Some(1), Duration::from_secs(1), false),
            After::Restart
        );
        assert_eq!(
            p.after(Some(1), Duration::from_secs(1), false),
            After::Restart
        );
        assert_eq!(
            p.after(Some(1), Duration::from_secs(1), false),
            After::GiveUp
        );
    }

    #[test]
    fn many_restarts_in_ten_minutes_give_up() {
        let mut p = Policy::new();
        for _ in 0..5 {
            assert_eq!(
                p.after(None, Duration::from_secs(60), false),
                After::Restart
            );
        }
        assert_eq!(p.after(None, Duration::from_secs(60), false), After::GiveUp);
    }
}
