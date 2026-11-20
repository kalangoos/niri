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

/// Whether `client` may see the global of `cap`.
pub fn allowed(dh: &DisplayHandle, client: &Client, cap: Capability) -> bool {
    let Some(path) = std::env::var_os("KALANGO_NIRI_CONTROLE") else {
        return true;
    };
    let Ok(cred) = client.get_credentials(dh) else {
        return false;
    };
    let owner: u32 = std::env::var("KALANGO_NIRI_CONTROLE_DONO").ok().and_then(|u| u.parse().ok()).unwrap_or(0);
    let Ok(meta) = std::fs::metadata(&path) else {
        return false;
    };
    // a file anyone else could have written is no permission
    if meta.uid() != owner || meta.mode() & 0o022 != 0 {
        warn!("remote control: {:?} is not owned by uid {owner} or is writable by others; denying", path);
        return false;
    }
    let Ok(text) = std::fs::read_to_string(&path) else {
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
}
