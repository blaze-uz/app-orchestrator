//! Unix (macOS / Linux) implementation of the [`crate::platform`] API.
//!
//! This is the original Karvon behaviour, unchanged: POSIX process groups via
//! `nix` (`killpg`, `setrlimit`) and process/port discovery via `ps` / `lsof`.
//! The stored `process_group_id` is the real pgid.

use super::{ExternalProcessRow, GroupError};
use crate::process_manager::display_command;
use nix::{
    errno::Errno,
    sys::resource::{setrlimit, Resource},
    sys::signal::{killpg, Signal},
    unistd::Pid,
};
use std::{
    collections::HashMap,
    io::{Error, ErrorKind},
    path::Path,
    process::Stdio,
};
use tokio::process::Command;

const STANDARD_PROCESS_PATHS: &[&str] = &[
    "/opt/homebrew/bin",
    "/usr/local/bin",
    "/usr/bin",
    "/bin",
    "/usr/sbin",
    "/sbin",
];

impl From<Errno> for GroupError {
    fn from(errno: Errno) -> Self {
        match errno {
            Errno::ESRCH => GroupError::NotFound,
            other => GroupError::Other(other.to_string()),
        }
    }
}

/// Put the spawned command in its own process group so shells and the workers
/// they fork can be terminated together (`process_group(0)` ⇒ pgid = child pid).
pub fn set_process_group(command: &mut Command) {
    command.process_group(0);
}

/// Cap the child's address space (`RLIMIT_AS`) before exec.
pub fn apply_memory_limit(command: &mut Command, limit_bytes: u64) {
    unsafe {
        command.pre_exec(move || {
            setrlimit(Resource::RLIMIT_AS, limit_bytes as _, limit_bytes as _)
                .map_err(|error| Error::new(ErrorKind::Other, error))?;
            Ok(())
        });
    }
}

/// Build a login-shell fallback command used when a bare command is not found on
/// `PATH` (so shell builtins / aliases / version managers still resolve).
pub fn shell_command(tokens: &[String]) -> Command {
    let mut command = Command::new("/bin/zsh");
    command
        .arg("-lc")
        .arg(format!("exec {}", display_command(tokens)));
    command
}

/// Compose the default `PATH` for managed processes: Herd's bin shim, the
/// standard macOS/Homebrew locations, then the inherited `PATH`.
pub fn default_process_path(inherited_path: Option<String>, home_dir: Option<String>) -> String {
    let mut paths = Vec::new();
    if let Some(home_dir) = home_dir {
        paths.push(format!("{home_dir}/Library/Application Support/Herd/bin"));
    }
    paths.extend(STANDARD_PROCESS_PATHS.iter().map(|path| path.to_string()));
    if let Some(inherited_path) = inherited_path {
        paths.extend(
            inherited_path
                .split(':')
                .map(str::trim)
                .filter(|path| !path.is_empty())
                .map(str::to_string),
        );
    }
    dedupe_paths(paths).join(":")
}

fn dedupe_paths(paths: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    paths
        .into_iter()
        .filter(|path| seen.insert(path.clone()))
        .collect()
}

/// Send `SIGTERM` to a process group. `Err(NotFound)` ⇒ group already gone.
pub fn terminate_group(process_group_id: u32) -> Result<(), GroupError> {
    killpg(Pid::from_raw(process_group_id as i32), Signal::SIGTERM).map_err(GroupError::from)
}

/// Send `SIGKILL` to a process group.
pub fn force_kill_group(process_group_id: u32) -> Result<(), GroupError> {
    killpg(Pid::from_raw(process_group_id as i32), Signal::SIGKILL).map_err(GroupError::from)
}

/// Probe whether a process group still exists (`killpg(pgid, 0)`).
pub fn group_exists(process_group_id: u32) -> bool {
    match killpg(Pid::from_raw(process_group_id as i32), None::<Signal>) {
        Ok(()) => true,
        Err(Errno::ESRCH) => false,
        Err(_) => true,
    }
}

/// A live process's `stat` is "live" unless it is a zombie (`Z`).
pub fn is_live_stat(stat: &str) -> bool {
    !stat.contains('Z')
}

pub async fn list_live_processes() -> Vec<ExternalProcessRow> {
    let output = Command::new("ps")
        .arg("-ax")
        .args(["-o", "pid="])
        .args(["-o", "pgid="])
        .args(["-o", "user="])
        .args(["-o", "rss="])
        .args(["-o", "pcpu="])
        .args(["-o", "etime="])
        .args(["-o", "stat="])
        .args(["-o", "lstart="])
        .args(["-o", "command="])
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
        .filter_map(parse_external_process_row)
        .filter(|row| is_live_stat(&row.stat))
        .collect()
}

fn parse_external_process_row(line: &str) -> Option<ExternalProcessRow> {
    let (pid, remainder) = take_process_token(line.trim_start())?;
    let (process_group_id, remainder) = take_process_token(remainder)?;
    let (user, remainder) = take_process_token(remainder)?;
    let (rss, remainder) = take_process_token(remainder)?;
    let (pcpu, remainder) = take_process_token(remainder)?;
    let (etime, remainder) = take_process_token(remainder)?;
    let (stat, remainder) = take_process_token(remainder)?;
    // lstart is a fixed 5-token timestamp like "Sat May 10 12:34:56 2026"
    let mut lstart_tokens: Vec<&str> = Vec::with_capacity(5);
    let mut cursor = remainder;
    for _ in 0..5 {
        let (token, rest) = take_process_token(cursor)?;
        lstart_tokens.push(token);
        cursor = rest;
    }
    Some(ExternalProcessRow {
        pid: pid.parse().ok()?,
        process_group_id: process_group_id.parse().ok()?,
        stat: stat.to_string(),
        command: cursor.trim_start().to_string(),
        user: user.to_string(),
        memory_kb: rss.parse().unwrap_or(0),
        cpu_percent: pcpu.parse().unwrap_or(0.0),
        etime: etime.to_string(),
        started_at: lstart_tokens.join(" "),
    })
}

fn take_process_token(input: &str) -> Option<(&str, &str)> {
    let input = input.trim_start();
    if input.is_empty() {
        return None;
    }
    let end = input.find(char::is_whitespace).unwrap_or(input.len());
    Some((&input[..end], &input[end..]))
}

pub async fn process_cwd(pid: u32) -> Option<String> {
    let output = Command::new("lsof")
        .arg("-a")
        .arg("-p")
        .arg(pid.to_string())
        .arg("-d")
        .arg("cwd")
        .arg("-Fn")
        .stderr(Stdio::null())
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .find_map(|line| line.strip_prefix('n').map(|value| value.to_string()))
}

pub async fn process_info_for_pid(pid: u32) -> Option<(u32, String)> {
    let output = Command::new("ps")
        .arg("-o")
        .arg("pgid=")
        .arg("-o")
        .arg("stat=")
        .arg("-p")
        .arg(pid.to_string())
        .stderr(Stdio::null())
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let output = String::from_utf8_lossy(&output.stdout);
    let mut parts = output.split_whitespace();
    let process_group_id = parts.next()?.parse::<u32>().ok()?;
    let stat = parts.next()?.to_string();
    Some((process_group_id, stat))
}

pub async fn live_process_in_group(process_group_id: u32) -> Option<u32> {
    let output = Command::new("ps")
        .arg("-ax")
        .arg("-o")
        .arg("pid=")
        .arg("-o")
        .arg("pgid=")
        .arg("-o")
        .arg("stat=")
        .stderr(Stdio::null())
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(parse_process_group_row)
        .find_map(|(pid, found_process_group_id, stat)| {
            if found_process_group_id == process_group_id && is_live_stat(&stat) {
                Some(pid)
            } else {
                None
            }
        })
}

fn parse_process_group_row(line: &str) -> Option<(u32, u32, String)> {
    let mut parts = line.split_whitespace();
    let pid = parts.next()?.parse::<u32>().ok()?;
    let process_group_id = parts.next()?.parse::<u32>().ok()?;
    let stat = parts.next()?.to_string();
    Some((pid, process_group_id, stat))
}

/// Find the TCP listener on `port`, returning `(pid, process_group_id, command)`.
pub async fn find_listener_on_port(port: u16) -> Option<(u32, u32, String)> {
    let output = Command::new("lsof")
        .arg(format!("-iTCP:{port}"))
        .arg("-sTCP:LISTEN")
        .arg("-P")
        .arg("-n")
        .arg("-Fpcg")
        .stderr(Stdio::null())
        .output()
        .await
        .ok()?;
    if output.stdout.is_empty() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).into_owned();

    let mut pid: Option<u32> = None;
    let mut pgid: Option<u32> = None;
    let mut command: Option<String> = None;

    for line in text.lines() {
        if let Some(rest) = line.strip_prefix('p') {
            if let (Some(p), Some(g)) = (pid, pgid) {
                return Some((p, g, command.clone().unwrap_or_default()));
            }
            pid = rest.trim().parse().ok();
            pgid = None;
            command = None;
        } else if let Some(rest) = line.strip_prefix('g') {
            pgid = rest.trim().parse().ok();
        } else if let Some(rest) = line.strip_prefix('c') {
            command = Some(rest.to_string());
        }
    }

    if let (Some(p), Some(g)) = (pid, pgid) {
        return Some((p, g, command.unwrap_or_default()));
    }
    None
}

pub async fn listening_ports_for_pids(pids: &[u32]) -> HashMap<u32, Vec<u32>> {
    if pids.is_empty() {
        return HashMap::new();
    }
    let pid_arg = pids
        .iter()
        .map(|p| p.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let output = Command::new("lsof")
        .arg("-iTCP")
        .arg("-sTCP:LISTEN")
        .arg("-P")
        .arg("-n")
        .arg("-Fpn")
        .arg("-p")
        .arg(pid_arg)
        .stderr(Stdio::null())
        .output()
        .await;
    let Ok(output) = output else {
        return HashMap::new();
    };
    if output.stdout.is_empty() {
        return HashMap::new();
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut result: HashMap<u32, Vec<u32>> = HashMap::new();
    let mut current_pid: Option<u32> = None;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix('p') {
            current_pid = rest.trim().parse().ok();
        } else if let Some(rest) = line.strip_prefix('n') {
            let Some(pid) = current_pid else {
                continue;
            };
            // rest looks like "*:8000" or "127.0.0.1:8765" or "[::1]:3000"
            let port_str = rest.rsplit(':').next().unwrap_or("");
            let Ok(port) = port_str.parse::<u32>() else {
                continue;
            };
            let entry = result.entry(pid).or_default();
            if !entry.contains(&port) {
                entry.push(port);
            }
        }
    }
    for ports in result.values_mut() {
        ports.sort_unstable();
    }
    result
}

pub async fn all_process_cwds() -> HashMap<u32, String> {
    let output = Command::new("lsof")
        .arg("-d")
        .arg("cwd")
        .arg("-Fpn")
        .stderr(Stdio::null())
        .output()
        .await;
    let Ok(output) = output else {
        return HashMap::new();
    };
    if output.stdout.is_empty() {
        return HashMap::new();
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut result = HashMap::new();
    let mut current_pid: Option<u32> = None;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix('p') {
            current_pid = rest.trim().parse().ok();
        } else if let Some(rest) = line.strip_prefix('n') {
            if let Some(pid) = current_pid {
                result.insert(pid, rest.to_string());
            }
        }
    }
    result
}

/// Read `(memory_bytes, cpu_percent)` for a single pid.
pub async fn read_process_metrics(pid: u32) -> Option<(u64, Option<f64>)> {
    let output = Command::new("ps")
        .arg("-o")
        .arg("rss=,pcpu=")
        .arg("-p")
        .arg(pid.to_string())
        .stderr(Stdio::null())
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let output = String::from_utf8_lossy(&output.stdout);
    let mut tokens = output.split_whitespace();
    let rss_kb = tokens.next()?.parse::<u64>().ok()?;
    let cpu_usage = tokens.next().and_then(|token| token.parse::<f64>().ok());
    Some((rss_kb.saturating_mul(1024), cpu_usage))
}

/// Open a file or folder in Finder.
pub fn open_path(path: &Path) -> std::io::Result<()> {
    std::process::Command::new("open").arg(path).spawn().map(|_| ())
}

/// Reveal (select) a file or folder in Finder.
pub fn reveal_path(path: &Path) -> std::io::Result<()> {
    std::process::Command::new("open")
        .arg("-R")
        .arg(path)
        .spawn()
        .map(|_| ())
}
