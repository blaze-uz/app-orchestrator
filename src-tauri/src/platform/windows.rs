//! Windows implementation of the [`crate::platform`] API.
//!
//! Windows has no POSIX process groups, so the supervision model is mapped as
//! follows:
//!
//! * **Spawn** — each command is launched with `CREATE_NEW_PROCESS_GROUP` (so
//!   the root child PID anchors a console process group) and `CREATE_NO_WINDOW`
//!   (so console children do not flash a window in front of the GUI app). The
//!   stored `process_group_id` is the **root child PID**.
//! * **Terminate / kill** — `taskkill /T` walks and kills the whole child tree
//!   by the root PID (`/F` forces). This is the Windows analogue of `killpg`.
//! * **Discovery** — `tasklist` (processes / liveness / memory) and
//!   `netstat -ano` (listening port → PID). The text parsers live in the parent
//!   module so they are unit-tested on every CI runner.
//!
//! ## Known behavioural caveats (vs. the unix path)
//!
//! 1. **No graceful SIGTERM.** `taskkill` without `/F` only posts `WM_CLOSE`
//!    (effective for GUI apps); console dev-servers are effectively force-killed.
//!    The stop-timeout/grace loop still runs unchanged, so behaviour is correct,
//!    just not "polite".
//! 2. **No hard memory cap.** `RLIMIT_AS` has no cheap, dependency-free Windows
//!    equivalent, so [`apply_memory_limit`] is a no-op. The live memory display
//!    and the app-level memory monitor still work via `tasklist`.
//! 3. **Degraded external-process matching.** `tasklist` exposes the image name
//!    (not the full command line) and not the working directory, so
//!    [`process_cwd`] returns `None` and adoption of externally-started
//!    processes matches on image name only.
//! 4. **`process_group_id` is the root PID**, so re-parented grandchildren can
//!    escape a tree kill in rare cases — acceptable for the dev-server workloads
//!    Karvon manages.

use super::{
    parse_netstat_listen_line, parse_tasklist_csv, ExternalProcessRow, GroupError,
};
use std::{collections::HashMap, path::Path, process::Stdio};
use tokio::process::Command;

/// New console process group, anchored at the child PID.
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
/// Do not allocate a console window for the child.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// `taskkill` exit code when the target PID does not exist.
const TASKKILL_NOT_FOUND: i32 = 128;

pub fn set_process_group(command: &mut Command) {
    command.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
}

/// No-op on Windows: there is no `RLIMIT_AS` equivalent without Job Objects.
/// The memory *monitor* still observes usage via `tasklist`; only the hard cap
/// is unavailable. See the module docs.
pub fn apply_memory_limit(_command: &mut Command, _limit_bytes: u64) {}

/// Fallback shell command (`cmd /C …`) used when a bare command is not found on
/// `PATH`, mirroring the unix login-shell fallback.
pub fn shell_command(tokens: &[String]) -> Command {
    let comspec = std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string());
    let mut command = Command::new(comspec);
    command.arg("/C").arg(super::windows_command_line(tokens));
    command
}

/// Default `PATH` for managed processes: Herd's bin shim then the inherited
/// `PATH`. Windows already resolves the system directories, so unlike the unix
/// build there is no hard-coded standard-paths list to prepend.
pub fn default_process_path(inherited_path: Option<String>, home_dir: Option<String>) -> String {
    let mut paths = Vec::new();
    let shim_root = std::env::var("LOCALAPPDATA").ok().or(home_dir);
    if let Some(root) = shim_root {
        paths.push(format!("{root}\\Herd\\bin"));
    }
    if let Some(inherited_path) = inherited_path {
        paths.extend(
            inherited_path
                .split(';')
                .map(str::trim)
                .filter(|path| !path.is_empty())
                .map(str::to_string),
        );
    }
    dedupe_paths(paths).join(";")
}

fn dedupe_paths(paths: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    paths
        .into_iter()
        .filter(|path| seen.insert(path.to_ascii_lowercase()))
        .collect()
}

fn taskkill(process_group_id: u32, force: bool) -> Result<(), GroupError> {
    let mut command = std::process::Command::new("taskkill");
    command
        .arg("/PID")
        .arg(process_group_id.to_string())
        .arg("/T");
    if force {
        command.arg("/F");
    }
    match command.output() {
        Ok(output) if output.status.success() => Ok(()),
        Ok(output) if output.status.code() == Some(TASKKILL_NOT_FOUND) => Err(GroupError::NotFound),
        Ok(output) => Err(GroupError::Other(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        )),
        Err(error) => Err(GroupError::Other(error.to_string())),
    }
}

/// Request a graceful tree termination (`taskkill /T`, no `/F`).
pub fn terminate_group(process_group_id: u32) -> Result<(), GroupError> {
    taskkill(process_group_id, false)
}

/// Force-kill the whole process tree (`taskkill /T /F`).
pub fn force_kill_group(process_group_id: u32) -> Result<(), GroupError> {
    taskkill(process_group_id, true)
}

/// Whether the root PID of the group is still present.
pub fn group_exists(process_group_id: u32) -> bool {
    pid_is_alive_blocking(process_group_id)
}

/// On Windows every listed process is live (there is no zombie state).
pub fn is_live_stat(_stat: &str) -> bool {
    true
}

fn pid_is_alive_blocking(pid: u32) -> bool {
    let output = std::process::Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"])
        .stderr(Stdio::null())
        .output();
    match output {
        Ok(output) if output.status.success() => String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(parse_tasklist_csv)
            .any(|row| row.pid == pid),
        _ => false,
    }
}

pub async fn list_live_processes() -> Vec<ExternalProcessRow> {
    let output = Command::new("tasklist")
        .args(["/V", "/FO", "CSV", "/NH"])
        .stderr(Stdio::null())
        .output()
        .await;
    let Ok(output) = output else {
        return vec![];
    };
    if !output.status.success() {
        return vec![];
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(parse_tasklist_csv)
        .map(|row| ExternalProcessRow {
            pid: row.pid,
            // No process groups on Windows: the pid is its own group.
            process_group_id: row.pid,
            stat: row.status,
            command: row.image,
            user: row.user,
            memory_kb: row.memory_kb,
            cpu_percent: 0.0,
            etime: String::new(),
            started_at: String::new(),
        })
        .collect()
}

pub async fn process_cwd(_pid: u32) -> Option<String> {
    // The working directory of an arbitrary process is not retrievable on
    // Windows without reading its PEB (privileged). Degrade gracefully.
    None
}

pub async fn process_info_for_pid(pid: u32) -> Option<(u32, String)> {
    let output = Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"])
        .stderr(Stdio::null())
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let alive = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(parse_tasklist_csv)
        .any(|row| row.pid == pid);
    // pgid == pid; report a synthetic running stat the live-check accepts.
    alive.then(|| (pid, "Running".to_string()))
}

pub async fn live_process_in_group(process_group_id: u32) -> Option<u32> {
    // The group id is the root pid; report it live if the root is still present.
    if pid_is_alive(process_group_id).await {
        Some(process_group_id)
    } else {
        None
    }
}

async fn pid_is_alive(pid: u32) -> bool {
    process_info_for_pid(pid).await.is_some()
}

pub async fn find_listener_on_port(port: u16) -> Option<(u32, u32, String)> {
    let listeners = netstat_listeners().await;
    let pid = listeners
        .into_iter()
        .find_map(|(listen_port, pid)| (listen_port == port).then_some(pid))?;
    let command = image_name_for_pid(pid).await.unwrap_or_default();
    // pgid == pid on Windows.
    Some((pid, pid, command))
}

pub async fn listening_ports_for_pids(pids: &[u32]) -> HashMap<u32, Vec<u32>> {
    if pids.is_empty() {
        return HashMap::new();
    }
    let wanted: std::collections::HashSet<u32> = pids.iter().copied().collect();
    let mut result: HashMap<u32, Vec<u32>> = HashMap::new();
    for (port, pid) in netstat_listeners().await {
        if !wanted.contains(&pid) {
            continue;
        }
        let entry = result.entry(pid).or_default();
        let port = u32::from(port);
        if !entry.contains(&port) {
            entry.push(port);
        }
    }
    for ports in result.values_mut() {
        ports.sort_unstable();
    }
    result
}

pub async fn all_process_cwds() -> HashMap<u32, String> {
    // See `process_cwd` — not retrievable on Windows.
    HashMap::new()
}

pub async fn read_process_metrics(pid: u32) -> Option<(u64, Option<f64>)> {
    let output = Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"])
        .stderr(Stdio::null())
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let row = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(parse_tasklist_csv)
        .find(|row| row.pid == pid)?;
    Some((row.memory_kb.saturating_mul(1024), None))
}

/// Open a file or folder in File Explorer.
pub fn open_path(path: &Path) -> std::io::Result<()> {
    std::process::Command::new("explorer").arg(path).spawn().map(|_| ())
}

/// Reveal (select) a file or folder in File Explorer.
pub fn reveal_path(path: &Path) -> std::io::Result<()> {
    // `explorer /select,<path>` highlights the item in its parent folder. Unlike
    // the address-bar resolver used by `explorer <path>`, the `/select` switch
    // does NOT auto-correct forward slashes, and Rust's `Display` prints them
    // verbatim on Windows — so a config path like `C:/Users/dev/proj` would make
    // Explorer ignore the selection and open the default folder. Normalize to
    // backslashes first. (`explorer` often exits non-zero even on success, so the
    // exit status is intentionally not inspected.)
    let native = path.display().to_string().replace('/', "\\");
    std::process::Command::new("explorer")
        .arg(format!("/select,{native}"))
        .spawn()
        .map(|_| ())
}

/// All TCP listeners as `(local_port, pid)` via `netstat -ano`.
///
/// The `-p TCP` filter is deliberately NOT used: on Windows `tcp` and `tcpv6`
/// are distinct protocol tokens, so `-p TCP` returns IPv4 only and drops every
/// IPv6 listener — and Node 17+/Vite bind `localhost` to `[::1]`/`[::]` only.
/// Plain `netstat -ano` labels both families `TCP` (IPv6 shown as `[..]:port`,
/// which the parser handles) and the `proto == TCP` guard rejects UDP rows.
async fn netstat_listeners() -> Vec<(u16, u32)> {
    let output = Command::new("netstat")
        .args(["-ano"])
        .stderr(Stdio::null())
        .output()
        .await;
    let Ok(output) = output else {
        return vec![];
    };
    if !output.status.success() {
        return vec![];
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(parse_netstat_listen_line)
        .collect()
}

async fn image_name_for_pid(pid: u32) -> Option<String> {
    let output = Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"])
        .stderr(Stdio::null())
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(parse_tasklist_csv)
        .find(|row| row.pid == pid)
        .map(|row| row.image)
}
