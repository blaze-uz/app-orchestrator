use crate::models::{Machine, MachineConnectionResult};
use crate::platform;
use std::{collections::HashMap, process::Stdio, time::Duration};
use tokio::{process::Command, time::Instant};

pub const REMOTE_PID_MARKER_PREFIX: &str = "__HERD_REMOTE_PID__=";

const CONNECT_TIMEOUT_SECS: u64 = 10;
const SERVER_ALIVE_INTERVAL_SECS: u64 = 30;
const SERVER_ALIVE_COUNT_MAX: u64 = 3;
const CONNECTION_TEST_TIMEOUT: Duration = Duration::from_secs(15);

pub fn build_ssh_command(
    machine: &Machine,
    user_command: &[String],
    cwd: Option<&str>,
    env: &HashMap<String, String>,
) -> Command {
    let mut command = Command::new("ssh");
    apply_common_options(&mut command, machine);
    command.arg(format!("{}@{}", machine.ssh_user, machine.hostname));
    command.arg(build_remote_payload(user_command, cwd, env));
    command.stdout(Stdio::piped()).stderr(Stdio::piped()).stdin(Stdio::null());
    platform::set_process_group(&mut command);
    command
}

pub async fn test_connection(machine: &Machine) -> MachineConnectionResult {
    let start = Instant::now();
    let mut command = Command::new("ssh");
    apply_common_options(&mut command, machine);
    command.arg(format!("{}@{}", machine.ssh_user, machine.hostname));
    command.arg("echo __HERD_PROBE_OK__:$(whoami):$(hostname)");
    command.stdout(Stdio::piped()).stderr(Stdio::piped()).stdin(Stdio::null());

    let attempt = tokio::time::timeout(CONNECTION_TEST_TIMEOUT, command.output()).await;
    let elapsed_ms = start.elapsed().as_millis().min(u128::from(u32::MAX)) as u32;

    match attempt {
        Ok(Ok(output)) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let probe = stdout
                .lines()
                .find_map(|line| line.trim().strip_prefix("__HERD_PROBE_OK__:"))
                .unwrap_or("");
            let mut parts = probe.split(':');
            let user = parts.next().unwrap_or("").trim();
            let host = parts.next().unwrap_or("").trim();
            let detail = if !user.is_empty() && !host.is_empty() {
                format!("Connected as {user}@{host}")
            } else {
                "Connected".to_string()
            };
            MachineConnectionResult {
                ok: true,
                latency_ms: elapsed_ms,
                detail,
            }
        }
        Ok(Ok(output)) => {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let detail = if stderr.is_empty() {
                format!("ssh exited with status {}", output.status)
            } else {
                classify_ssh_failure(&stderr, output.status.code())
            };
            MachineConnectionResult { ok: false, latency_ms: elapsed_ms, detail }
        }
        Ok(Err(err)) => MachineConnectionResult {
            ok: false,
            latency_ms: elapsed_ms,
            detail: if err.kind() == std::io::ErrorKind::NotFound {
                "ssh binary not found on PATH — install openssh-client".to_string()
            } else {
                format!("Unable to invoke ssh: {err}")
            },
        },
        Err(_) => MachineConnectionResult {
            ok: false,
            latency_ms: elapsed_ms,
            detail: format!(
                "Connection attempt timed out after {} seconds",
                CONNECTION_TEST_TIMEOUT.as_secs()
            ),
        },
    }
}

pub async fn run_remote_command(
    machine: &Machine,
    shell_command: &str,
) -> Result<std::process::Output, std::io::Error> {
    let mut command = Command::new("ssh");
    apply_common_options(&mut command, machine);
    command.arg(format!("{}@{}", machine.ssh_user, machine.hostname));
    command.arg(shell_command);
    command.stdout(Stdio::piped()).stderr(Stdio::piped()).stdin(Stdio::null());
    command.output().await
}

pub async fn kill_remote_process(machine: &Machine, remote_pid: u32, signal: &str) -> Result<(), String> {
    // Allowlist to prevent shell injection through the signal argument: it is
    // interpolated into a remote shell command, so an attacker-controlled value
    // could otherwise break out of the `kill -…` form.
    let safe_signal = match signal {
        "TERM" | "KILL" | "INT" | "HUP" | "QUIT" | "USR1" | "USR2" | "STOP" | "CONT" => signal,
        other => return Err(format!("disallowed signal: {other}")),
    };
    let mut command = Command::new("ssh");
    apply_common_options(&mut command, machine);
    command.arg(format!("{}@{}", machine.ssh_user, machine.hostname));
    command.arg(format!("kill -{safe_signal} {remote_pid} 2>/dev/null || true"));
    command.stdout(Stdio::null()).stderr(Stdio::piped()).stdin(Stdio::null());
    match tokio::time::timeout(Duration::from_secs(10), command.status()).await {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(err)) => Err(err.to_string()),
        Err(_) => Err("ssh kill timed out".to_string()),
    }
}

pub fn classify_ssh_failure(stderr: &str, exit_code: Option<i32>) -> String {
    let trimmed = stderr.trim();
    let lower = trimmed.to_lowercase();
    if lower.contains("permission denied") {
        format!("SSH auth failed: {trimmed}")
    } else if lower.contains("could not resolve")
        || lower.contains("name or service not known")
        || lower.contains("connection timed out")
        || lower.contains("no route to host")
        || lower.contains("network is unreachable")
        || lower.contains("connection refused")
    {
        format!("Host unreachable: {trimmed}")
    } else if exit_code == Some(127) || lower.contains("command not found") {
        format!("Remote command not found: {trimmed}")
    } else if trimmed.is_empty() {
        format!("ssh failed (exit {exit_code:?})")
    } else {
        trimmed.to_string()
    }
}

pub fn parse_remote_pid_marker(line: &str) -> Option<u32> {
    let trimmed = line.trim().trim_matches(|c: char| c == '\r' || c == '\n');
    trimmed
        .strip_prefix(REMOTE_PID_MARKER_PREFIX)
        .and_then(|value| value.trim().parse::<u32>().ok())
}

fn apply_common_options(command: &mut Command, machine: &Machine) {
    command
        .arg("-o")
        .arg("BatchMode=yes")
        .arg("-o")
        .arg(format!("ConnectTimeout={CONNECT_TIMEOUT_SECS}"))
        .arg("-o")
        .arg(format!("ServerAliveInterval={SERVER_ALIVE_INTERVAL_SECS}"))
        .arg("-o")
        .arg(format!("ServerAliveCountMax={SERVER_ALIVE_COUNT_MAX}"))
        .arg("-o")
        .arg("StrictHostKeyChecking=accept-new")
        .arg("-o")
        .arg("LogLevel=ERROR")
        .arg("-tt")
        .arg("-p")
        .arg(machine.ssh_port.to_string());
    if let Some(key_path) = machine
        .ssh_key_path
        .as_ref()
        .map(|path| path.trim())
        .filter(|path| !path.is_empty())
    {
        command.arg("-i").arg(key_path);
    }
}

fn build_remote_payload(
    user_command: &[String],
    cwd: Option<&str>,
    env: &HashMap<String, String>,
) -> String {
    let mut script = String::new();
    script.push_str("printf '");
    script.push_str(REMOTE_PID_MARKER_PREFIX);
    script.push_str("%s\\n' \"$$\" 1>&2");
    if let Some(cwd) = cwd.map(str::trim).filter(|cwd| !cwd.is_empty()) {
        script.push_str(" && cd ");
        script.push_str(&shell_single_quote(cwd));
    }
    if !env.is_empty() {
        let mut keys: Vec<&String> = env.keys().collect();
        keys.sort();
        script.push_str(" && export");
        for key in keys {
            if !is_valid_env_key(key) {
                continue;
            }
            let value = env.get(key).map(String::as_str).unwrap_or("");
            script.push(' ');
            script.push_str(&shell_single_quote(&format!("{key}={value}")));
        }
    }
    script.push_str(" && exec ");
    script.push_str(
        &user_command
            .iter()
            .map(|token| shell_single_quote(token))
            .collect::<Vec<_>>()
            .join(" "),
    );
    script
}

fn shell_single_quote(value: &str) -> String {
    if value.is_empty() {
        return "''".to_string();
    }
    if value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/' | ':' | '@' | '%' | '=' | '+'))
    {
        return value.to_string();
    }
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn is_valid_env_key(key: &str) -> bool {
    if key.is_empty() {
        return false;
    }
    let mut chars = key.chars();
    let first = chars.next().unwrap();
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_remote_pid_marker() {
        assert_eq!(parse_remote_pid_marker("__HERD_REMOTE_PID__=12345"), Some(12345));
        assert_eq!(parse_remote_pid_marker("__HERD_REMOTE_PID__=12345\r"), Some(12345));
        assert_eq!(parse_remote_pid_marker("   __HERD_REMOTE_PID__=12345   "), Some(12345));
        assert!(parse_remote_pid_marker("hello world").is_none());
        assert!(parse_remote_pid_marker("__HERD_REMOTE_PID__=abc").is_none());
    }

    #[test]
    fn quotes_special_characters() {
        assert_eq!(shell_single_quote("foo"), "foo");
        assert_eq!(shell_single_quote("hello world"), "'hello world'");
        assert_eq!(shell_single_quote("it's"), "'it'\\''s'");
        assert_eq!(shell_single_quote(""), "''");
    }

    #[test]
    fn builds_remote_payload_with_cwd_and_env() {
        let env: HashMap<String, String> = [
            ("FOO".to_string(), "bar baz".to_string()),
            ("LOG".to_string(), "debug".to_string()),
        ]
        .into_iter()
        .collect();
        let payload = build_remote_payload(
            &["node".to_string(), "server.js".to_string()],
            Some("/srv/app"),
            &env,
        );
        assert!(payload.starts_with("printf '__HERD_REMOTE_PID__=%s\\n' \"$$\" 1>&2"));
        assert!(payload.contains("cd /srv/app"));
        assert!(payload.contains("export 'FOO=bar baz' LOG=debug"));
        assert!(payload.contains("exec node server.js"));
    }

    #[test]
    fn ignores_invalid_env_keys() {
        let env: HashMap<String, String> =
            [("0BAD".to_string(), "x".to_string()), ("OK".to_string(), "1".to_string())]
                .into_iter()
                .collect();
        let payload = build_remote_payload(&["sh".to_string()], None, &env);
        assert!(payload.contains("export OK=1"));
        assert!(!payload.contains("0BAD"));
    }
}
