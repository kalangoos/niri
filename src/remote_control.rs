//! KalangoOS: who may capture the screen and inject input.
//!
//! wlr-screencopy, the wlr virtual pointer and the virtual keyboard let a client see everything and act as the
//! person. Upstream offers them to every unsandboxed client. With `KALANGO_NIRI_CONTROLE` set (the KalangoOS
//! session sets it), only the clients that file allows see them. The file is written by kalangod (root) — a
//! program in the session cannot grant itself — and read on every check, so a grant or a revocation takes effect
//! for the next connection. Lines:
//!
//!   exe <path> <caps>              every process of that executable (the shell's on-screen keyboard)
//!   pid <pid> <start-time> <caps>  one process (start time from /proc/<pid>/stat, so a reused pid is not it)
//!
//! `<caps>` is a comma list of `captura`, `ponteiro`, `teclado`. The file must belong to root, or to the uid in
//! `KALANGO_NIRI_CONTROLE_DONO` (development sessions, where kalangod runs as the person).

use std::os::unix::fs::MetadataExt;

use smithay::reexports::wayland_server::{Client, DisplayHandle};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    Screencopy,
    Pointer,
    Keyboard,
}

impl Capability {
    fn word(self) -> &'static str {
        match self {
            Capability::Screencopy => "captura",
            Capability::Pointer => "ponteiro",
            Capability::Keyboard => "teclado",
        }
    }
}

/// The process as the kernel describes it.
pub struct Process {
    pub pid: i32,
    pub exe: String,
    pub start_time: u64,
}

/// Whether the allow file's text lets this process use `cap`.
pub fn permits(text: &str, p: &Process, cap: Capability) -> bool {
    text.lines().any(|line| {
        let f: Vec<&str> = line.split_whitespace().collect();
        let (matches, caps) = match f.as_slice() {
            ["exe", path, caps] => (!p.exe.is_empty() && *path == p.exe, *caps),
            ["pid", pid, start, caps] => (pid.parse() == Ok(p.pid) && start.parse() == Ok(p.start_time), *caps),
            _ => return false,
        };
        matches && caps.split(',').any(|c| c == cap.word())
    })
}

fn start_time(pid: i32) -> Option<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // the name may hold spaces: the fields start after the last ')'; starttime is field 22
    stat.rsplit_once(')')?.1.split_whitespace().nth(19)?.parse().ok()
}

/// The allow file's text, only when the file is one kalangod (root, or the development owner) wrote and nobody
/// else can write: a file anyone else could have written is no permission.
fn trusted_text(path: &std::ffi::OsStr) -> Option<String> {
    let owner: u32 = std::env::var("KALANGO_NIRI_CONTROLE_DONO").ok().and_then(|u| u.parse().ok()).unwrap_or(0);
    let meta = std::fs::metadata(path).ok()?;
    if meta.uid() != owner || meta.mode() & 0o022 != 0 {
        warn!("remote control: {:?} is not owned by uid {owner} or is writable by others; denying", path);
        return None;
    }
    std::fs::read_to_string(path).ok()
}

/// Whether `client` may see the global of `cap`.
pub fn allowed(dh: &DisplayHandle, client: &Client, cap: Capability) -> bool {
    let Some(path) = std::env::var_os("KALANGO_NIRI_CONTROLE") else {
        return true;
    };
    let Ok(cred) = client.get_credentials(dh) else {
        return false;
    };
    let Some(text) = trusted_text(&path) else {
        return false;
    };
    let p = Process {
        pid: cred.pid,
        exe: std::fs::read_link(format!("/proc/{}/exe", cred.pid))
            .map(|e| e.display().to_string())
            .unwrap_or_default(),
        start_time: start_time(cred.pid).unwrap_or(0),
    };
    let ok = permits(&text, &p, cap);
    debug!("remote control: pid {} ({}) {:?} → {ok}", p.pid, p.exe, cap);
    ok
}

/// Whether the allow file gives anything to a single process: a remote session is going on.
pub fn in_session(text: &str) -> bool {
    text.lines().any(|line| line.split_whitespace().next() == Some("pid"))
}

/// Whether a remote session is going on in this compositor (see `in_session`).
pub fn session_active() -> bool {
    std::env::var_os("KALANGO_NIRI_CONTROLE")
        .and_then(|path| std::fs::read_to_string(path).ok())
        .is_some_and(|text| in_session(&text))
}

/// Whether an agent controls the screen: a grant by pid with the word `agente` among its capabilities (kalangod
/// writes it for the screen helper of an agent's session; Kalango Remote's grants do not have it).
pub fn agent_in_session(text: &str) -> bool {
    text.lines().any(|line| {
        let f: Vec<&str> = line.split_whitespace().collect();
        matches!(f.as_slice(), ["pid", _, _, caps] if caps.split(',').any(|c| c == "agente"))
    })
}

/// Whether an agent controls the screen of this compositor now (see `agent_in_session`).
pub fn agent_session_active() -> bool {
    std::env::var_os("KALANGO_NIRI_CONTROLE")
        .and_then(|path| std::fs::read_to_string(path).ok())
        .is_some_and(|text| agent_in_session(&text))
}

/// Privacy mode: a grant by pid with the word `privacidade` among its capabilities (Kalango Remote asks for it only
/// for a session nobody on this computer approved), whose process is still the one that was granted and whose parent
/// is still Kalango Remote's app. `lookup(pid, start)` says that: the app's pid (the parent of the granted core) or
/// None. The local monitors then show a black cover while the screen captures (wlr-screencopy, screencasts) keep
/// showing the real desktop.
///
/// What fails open (the person is never locked out, the cover goes away): the allow file unreadable, not the
/// kalangod's (owner, or writable by others), without the word, with a malformed line; the core dead or its pid reused;
/// the app (its parent) dead; no mapped `kalango-remote-banner` layer owned by that app on any output (see
/// `Niri::kalango_privacy_covering`); the session locked. What fails closed: nothing but a live grant with a live app
/// and a live banner. Warnings are logged for the unreadable and malformed cases.
pub fn privacy_grant(text: &str, lookup: impl Fn(i32, u64) -> Option<i32>) -> Option<i32> {
    text.lines().find_map(|line| {
        let f: Vec<&str> = line.split_whitespace().collect();
        match f.as_slice() {
            ["pid", pid, start, caps] if caps.split(',').any(|c| c == "privacidade") => {
                match (pid.parse(), start.parse()) {
                    (Ok(p), Ok(s)) => lookup(p, s),
                    _ => None,
                }
            }
            _ => None,
        }
    })
}

/// Lines of the allow file that start like a grant by pid but cannot be read (they would grant nothing).
pub fn malformed_grants(text: &str) -> usize {
    text.lines()
        .filter(|line| {
            let f: Vec<&str> = line.split_whitespace().collect();
            f.first() == Some(&"pid")
                && !matches!(f.as_slice(), ["pid", pid, start, _] if pid.parse::<i32>().is_ok() && start.parse::<u64>().is_ok())
        })
        .count()
}

/// The app that owns the granted core: the parent of process `pid` (started at `start`), provided it is the
/// `kalango-remote` next to the core's own executable. The pid of that app is what the layers of Kalango Remote must
/// belong to.
pub fn core_app_pid(pid: i32, start: u64) -> Option<i32> {
    if start_time(pid) != Some(start) {
        return None;
    }
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let ppid: i32 = stat.rsplit_once(')')?.1.split_whitespace().nth(1)?.parse().ok()?;
    if ppid <= 1 {
        return None;
    }
    let core = std::fs::read_link(format!("/proc/{pid}/exe")).ok()?;
    let app = std::fs::read_link(format!("/proc/{ppid}/exe")).ok()?;
    (app == core.with_file_name("kalango-remote")).then_some(ppid)
}

/// What the allow file says about privacy mode now.
pub enum Privacy {
    /// Not wanted (no file, no variable, no grant with the word, a dead core or app).
    Off,
    /// Wanted; the layers of this app pid are the ones to trust.
    On(i32),
    /// The file is there but cannot be trusted or read.
    Unreadable,
}

pub fn privacy_state() -> Privacy {
    let Some(path) = std::env::var_os("KALANGO_NIRI_CONTROLE") else {
        return Privacy::Off;
    };
    if !std::path::Path::new(&path).exists() {
        return Privacy::Off;
    }
    let Some(text) = trusted_text(&path) else {
        return Privacy::Unreadable;
    };
    match privacy_grant(&text, core_app_pid) {
        Some(app) => Privacy::On(app),
        None => Privacy::Off,
    }
}

/// What this compositor can do beyond upstream niri, for `niri msg kalango-features` (Kalango Remote enables its
/// privacy switch only when the list has `privacy`). Capabilities that need the allow file are listed only when
/// the session set `KALANGO_NIRI_CONTROLE`.
pub fn features() -> Vec<String> {
    let mut v = Vec::new();
    if std::env::var_os("KALANGO_NIRI_CONTROLE").is_some() {
        v.push(String::from("remote-control"));
        v.push(String::from("privacy"));
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grants_by_exe_and_by_pid() {
        let text = "exe /usr/bin/kalango-shell teclado\npid 4242 987654 captura,ponteiro,teclado\nlixo\n";
        let shell = Process { pid: 10, exe: "/usr/bin/kalango-shell".into(), start_time: 1 };
        assert!(permits(text, &shell, Capability::Keyboard));
        assert!(!permits(text, &shell, Capability::Screencopy), "the shell only types");
        let core = Process { pid: 4242, exe: "/usr/bin/kalango-remote-core".into(), start_time: 987654 };
        assert!(permits(text, &core, Capability::Screencopy));
        assert!(permits(text, &core, Capability::Pointer));
        let reused = Process { start_time: 1, ..core };
        assert!(!permits(text, &reused, Capability::Screencopy), "a reused pid is another process");
        let other = Process { pid: 7, exe: "/usr/bin/grim".into(), start_time: 3 };
        assert!(!permits(text, &other, Capability::Screencopy));
        assert!(!permits("", &other, Capability::Keyboard));
        let no_exe = Process { pid: 8, exe: String::new(), start_time: 0 };
        assert!(!permits("exe  teclado", &no_exe, Capability::Keyboard));
    }

    #[test]
    fn a_session_is_a_grant_by_pid() {
        assert!(!in_session("exe /usr/bin/kalango-shell teclado\n"));
        assert!(in_session("exe /usr/bin/kalango-shell teclado\npid 4242 987654 captura\n"));
    }

    #[test]
    fn an_agent_session_is_marked() {
        assert!(!agent_in_session("pid 4242 987654 captura,ponteiro,teclado\n"), "Kalango Remote's");
        assert!(agent_in_session("exe /usr/bin/kalango-shell teclado\npid 7 9 captura,ponteiro,teclado,agente\n"));
        assert!(!agent_in_session("exe /usr/bin/x captura,agente\n"), "only a grant to one process");
    }

    #[test]
    fn privacy_needs_the_word_the_same_process_and_its_app() {
        let lookup = |pid: i32, start: u64| (pid == 4242 && start == 987654).then_some(555);
        assert_eq!(privacy_grant("pid 4242 987654 captura,ponteiro,teclado,privacidade\n", lookup), Some(555));
        assert_eq!(privacy_grant("pid 4242 987654 captura,ponteiro,teclado\n", lookup), None, "no word, no cover");
        assert_eq!(privacy_grant("pid 4242 1 captura,privacidade\n", lookup), None, "a reused pid is another process");
        assert_eq!(privacy_grant("pid 77 987654 privacidade\n", lookup), None, "the core died");
        assert_eq!(privacy_grant("exe /usr/bin/x captura,privacidade\n", lookup), None, "only a grant to one process");
        assert_eq!(privacy_grant("", lookup), None);
        // the app died (the lookup finds no app): the cover goes even though the core lives
        assert_eq!(privacy_grant("pid 4242 987654 privacidade\n", |_, _| None), None);
    }

    #[test]
    fn grants_that_cannot_be_read_are_counted() {
        assert_eq!(malformed_grants("pid 1 2 captura\npid x 2 captura\npid 1 captura\nexe /x captura\npid 3 4 a,b"), 2);
    }
}
