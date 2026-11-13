//! Process memory on macOS: the footprint the task manager and experiment 14
//! report, and the WebKit processes a profile process is responsible for.

use serde::Serialize;

unsafe extern "C" {
    // libsystem's answer to "which app is this process working for": the
    // app for its WebKit XPC services. Declared in <libproc.h>'s private
    // companion; present since macOS 10.14.
    fn responsibility_get_pid_responsible_for_pid(pid: libc::pid_t) -> libc::pid_t;
}

/// `phys_footprint`, the number Activity Monitor shows as Memory.
pub fn footprint(pid: i32) -> Option<u64> {
    let mut info = std::mem::MaybeUninit::<libc::rusage_info_v4>::zeroed();
    // SAFETY: the buffer is a rusage_info_v4 and the flavor says so.
    let rc = unsafe { libc::proc_pid_rusage(pid, libc::RUSAGE_INFO_V4, info.as_mut_ptr().cast()) };
    // SAFETY: proc_pid_rusage filled the struct when it returned 0.
    (rc == 0).then(|| unsafe { info.assume_init() }.ri_phys_footprint)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Process {
    pub pid: i32,
    pub name: String,
    pub footprint: Option<u64>,
}

fn path_of(pid: i32) -> Option<String> {
    let mut buf = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    // SAFETY: the buffer is PROC_PIDPATHINFO_MAXSIZE bytes long.
    let len = unsafe { libc::proc_pidpath(pid, buf.as_mut_ptr().cast(), buf.len() as u32) };
    (len > 0).then(|| String::from_utf8_lossy(&buf[..len as usize]).into_owned())
}

/// This process and its WebKit services (WebContent, Networking, GPU), with
/// their footprints.
///
/// macOS makes the app that launched a process responsible for it: Mado
/// started from Finder is responsible for its WebKit services, but started
/// from a terminal, the terminal is responsible for both. So a WebKit
/// service counts when its responsible process is this app's responsible
/// process. Run one prototype at a time from a terminal.
pub fn family() -> Vec<Process> {
    let me = std::process::id() as i32;
    // SAFETY: plain syscall wrapper on a valid PID.
    let my_responsible = unsafe { responsibility_get_pid_responsible_for_pid(me) };
    // SAFETY: a null buffer asks for the count.
    let count = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
    let mut pids = vec![0 as libc::pid_t; usize::try_from(count).unwrap_or(0) + 64];
    let bytes = i32::try_from(pids.len() * std::mem::size_of::<libc::pid_t>()).unwrap_or(i32::MAX);
    // SAFETY: the buffer holds `bytes` bytes of pid_t.
    let n = unsafe { libc::proc_listallpids(pids.as_mut_ptr().cast(), bytes) };
    pids.truncate(usize::try_from(n).unwrap_or(0));
    pids.into_iter()
        .filter_map(|pid| {
            let name = path_of(pid)
                .and_then(|p| p.rsplit('/').next().map(str::to_owned))
                .unwrap_or_default();
            // SAFETY: plain syscall wrapper on a PID from the kernel's list.
            let responsible = unsafe { responsibility_get_pid_responsible_for_pid(pid) };
            let ours = pid == me
                || responsible == me
                || (responsible == my_responsible && name.starts_with("com.apple.WebKit."));
            ours.then(|| Process {
                pid,
                name,
                footprint: footprint(pid),
            })
        })
        .collect()
}
