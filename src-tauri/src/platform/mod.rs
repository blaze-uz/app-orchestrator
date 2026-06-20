//! Platform abstraction for process supervision, discovery, and OS integration.
//!
//! Karvon supervises local dev processes by putting each managed command in its
//! own *process group* and terminating / probing the whole group together. That
//! model is POSIX-native (`killpg`, `setrlimit`, `ps`, `lsof`). This module hides
//! the OS-specific implementation behind one API so the rest of the backend is
//! platform-neutral:
//!
//! * **unix** ([`unix`]) — keeps the original `nix` + `ps`/`lsof` behaviour
//!   byte-for-byte. The stored `process_group_id` is the real POSIX pgid.
//! * **windows** ([`windows`]) — there is no pgid. A command is spawned with
//!   `CREATE_NEW_PROCESS_GROUP`, the **root child PID** is used as the
//!   `process_group_id`, the tree is killed with `taskkill /T`, and discovery
//!   uses `tasklist` / `netstat -ano`. See that module for the behavioural
//!   caveats (no graceful SIGTERM, no hard memory cap, degraded cwd matching).
//!
//! The persisted `process_group_id` is a plain `u32` on both platforms, so there
//! is no storage-schema or serde change.

use std::fmt;

#[cfg(unix)]
mod unix;
#[cfg(unix)]
pub use unix::*;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::*;

/// A row describing an OS process, normalised across platforms. On Windows the
/// `process_group_id` equals the `pid` (there are no process groups) and some
/// fields are best-effort (`cpu_percent`/`etime`/`started_at` may be empty).
#[derive(Clone, Debug)]
pub struct ExternalProcessRow {
    pub pid: u32,
    pub process_group_id: u32,
    // Only the unix zombie filter reads `stat`; on Windows `tasklist` lists only
    // live processes, so the field is populated for parity but never read.
    #[cfg_attr(windows, allow(dead_code))]
    pub stat: String,
    pub command: String,
    pub user: String,
    pub memory_kb: u64,
    pub cpu_percent: f32,
    pub etime: String,
    pub started_at: String,
}

/// Result of signalling / probing a process group, abstracted away from the
/// unix `Errno`. `NotFound` is the cross-platform spelling of `ESRCH` (no such
/// group) and lets callers treat the group as already gone.
#[derive(Debug)]
pub enum GroupError {
    NotFound,
    Other(String),
}

impl fmt::Display for GroupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GroupError::NotFound => write!(f, "no such process group"),
            GroupError::Other(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for GroupError {}

// ---------------------------------------------------------------------------
// Host-portable parsers
//
// These parse Windows `netstat` / `tasklist` output. They are compiled on Windows
// (where they are used) and in any `test` build (so their unit tests run in CI on
// the macOS/Linux runners too) — the parsers are the only piece of the Windows
// path that can be verified without a Windows machine. The `cfg(any(windows,
// test))` gate keeps them out of the non-test macOS build (where they would be
// dead code).
// ---------------------------------------------------------------------------

/// One TCP listener parsed from a `netstat -ano -p TCP` line: `(local_port, pid)`.
///
/// A listening socket is identified locale-independently by a wildcard foreign
/// address (`0.0.0.0:0`, `[::]:0`, or `*:*`) — established connections have a
/// real foreign address — so this does not depend on the localised `LISTENING`
/// word that `netstat` prints in non-English locales.
#[cfg(any(windows, test))]
pub(crate) fn parse_netstat_listen_line(line: &str) -> Option<(u16, u32)> {
    let mut parts = line.split_whitespace();
    let proto = parts.next()?;
    if !proto.eq_ignore_ascii_case("TCP") {
        return None;
    }
    let local = parts.next()?;
    let foreign = parts.next()?;
    // Columns 4 (state) and 5 (pid) for TCP. `next()` of remaining gives state.
    let state = parts.next()?;
    let pid_token = parts.next().unwrap_or(state);

    let foreign_is_wildcard = foreign_port(foreign) == Some(0);
    let state_is_listen = state.to_ascii_uppercase().starts_with("LISTEN");
    if !(foreign_is_wildcard || state_is_listen) {
        return None;
    }

    let port = port_suffix(local)?;
    let pid = pid_token.parse::<u32>().ok()?;
    Some((port, pid))
}

/// Extract the port from an `ADDR:PORT` token, handling IPv6 `[::1]:3000`.
#[cfg(any(windows, test))]
fn port_suffix(addr: &str) -> Option<u16> {
    addr.rsplit(':').next()?.parse::<u16>().ok()
}

#[cfg(any(windows, test))]
fn foreign_port(addr: &str) -> Option<u16> {
    port_suffix(addr)
}

/// A row of `tasklist /FO CSV` output. Field count varies: the plain form has 5
/// columns (image, pid, session, session#, mem); the `/V` (verbose) form adds
/// status, user, cpu-time, window-title.
#[cfg(any(windows, test))]
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TasklistRow {
    pub image: String,
    pub pid: u32,
    pub memory_kb: u64,
    pub status: String,
    pub user: String,
}

/// Parse one CSV line of `tasklist /FO CSV`. Returns `None` for the header or
/// malformed lines. Quote-aware so the thousands separator inside `"123,456 K"`
/// does not split the memory field.
#[cfg(any(windows, test))]
pub(crate) fn parse_tasklist_csv(line: &str) -> Option<TasklistRow> {
    let fields = split_csv_fields(line);
    if fields.len() < 5 {
        return None;
    }
    let pid = fields[1].trim().parse::<u32>().ok()?;
    let memory_kb = parse_mem_usage_kb(&fields[4]).unwrap_or(0);
    let status = fields.get(5).cloned().unwrap_or_default();
    let user = fields.get(6).cloned().unwrap_or_default();
    Some(TasklistRow {
        image: fields[0].clone(),
        pid,
        memory_kb,
        status,
        user,
    })
}

/// Split a single CSV record into fields, honouring double-quoted fields. The
/// `tasklist` format only ever double-quotes fields and never embeds escaped
/// quotes, so this minimal splitter is sufficient.
#[cfg(any(windows, test))]
fn split_csv_fields(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    for ch in line.chars() {
        match ch {
            '"' => in_quotes = !in_quotes,
            ',' if !in_quotes => {
                fields.push(std::mem::take(&mut current));
            }
            _ => current.push(ch),
        }
    }
    fields.push(current);
    fields
}

/// Join argument tokens into a single `cmd /C` command line.
///
/// Tokens containing whitespace, a quote, or a cmd.exe metacharacter are wrapped
/// in double quotes. Inside a quoted token a literal quote is represented by
/// doubling it (`""`) — that is cmd.exe's convention, *not* the C-runtime's
/// backslash-escape (`\"`), which cmd does not honour. This is only the fallback
/// path used when a bare command is not directly launchable (e.g. `npm`, which
/// is `npm.cmd` and needs `PATHEXT` resolution); typical dev commands have no
/// special characters and pass through unquoted.
#[cfg(any(windows, test))]
pub(crate) fn windows_command_line(tokens: &[String]) -> String {
    tokens
        .iter()
        .map(|token| {
            let needs_quote = token.is_empty()
                || token.chars().any(|c| {
                    c.is_whitespace() || matches!(c, '&' | '|' | '<' | '>' | '^' | '(' | ')' | '"')
                });
            if needs_quote {
                format!("\"{}\"", token.replace('"', "\"\""))
            } else {
                token.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Parse a `tasklist` memory-usage cell such as `"123,456 K"` into kilobytes.
#[cfg(any(windows, test))]
pub(crate) fn parse_mem_usage_kb(cell: &str) -> Option<u64> {
    let digits: String = cell.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    digits.parse::<u64>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ipv4_listener() {
        assert_eq!(
            parse_netstat_listen_line("  TCP    127.0.0.1:8000   0.0.0.0:0   LISTENING   12345"),
            Some((8000, 12345))
        );
    }

    #[test]
    fn parses_ipv6_listener() {
        assert_eq!(
            parse_netstat_listen_line("  TCP    [::]:445   [::]:0   LISTENING   4"),
            Some((445, 4))
        );
    }

    #[test]
    fn parses_listener_in_non_english_locale() {
        // German Windows prints "ABHÖREN" for LISTENING; the wildcard foreign
        // address still identifies it.
        assert_eq!(
            parse_netstat_listen_line("  TCP    0.0.0.0:135   0.0.0.0:0   ABHÖREN   968"),
            Some((135, 968))
        );
    }

    #[test]
    fn ignores_established_connection() {
        assert_eq!(
            parse_netstat_listen_line("  TCP    127.0.0.1:51000   140.82.112.3:443   ESTABLISHED   7777"),
            None
        );
    }

    #[test]
    fn ignores_header_and_udp() {
        assert_eq!(parse_netstat_listen_line("  Proto  Local Address  Foreign Address  State  PID"), None);
        assert_eq!(parse_netstat_listen_line("  UDP    0.0.0.0:5353   *:*   123"), None);
    }

    #[test]
    fn parses_verbose_tasklist_row() {
        let line = r#""node.exe","12345","Console","1","123,456 K","Running","DESKTOP-AB\dev","0:00:12","N/A""#;
        let row = parse_tasklist_csv(line).expect("row");
        assert_eq!(row.image, "node.exe");
        assert_eq!(row.pid, 12345);
        assert_eq!(row.memory_kb, 123456);
        assert_eq!(row.status, "Running");
        assert_eq!(row.user, "DESKTOP-AB\\dev");
    }

    #[test]
    fn parses_plain_tasklist_row_without_status() {
        let line = r#""svchost.exe","968","Services","0","8,200 K""#;
        let row = parse_tasklist_csv(line).expect("row");
        assert_eq!(row.pid, 968);
        assert_eq!(row.memory_kb, 8200);
        assert_eq!(row.status, "");
        assert_eq!(row.user, "");
    }

    #[test]
    fn rejects_malformed_tasklist_line() {
        assert!(parse_tasklist_csv("INFO: No tasks are running which match the specified criteria.").is_none());
        assert!(parse_tasklist_csv("").is_none());
    }

    #[test]
    fn parses_memory_cell() {
        assert_eq!(parse_mem_usage_kb("123,456 K"), Some(123456));
        assert_eq!(parse_mem_usage_kb("8,200 K"), Some(8200));
        assert_eq!(parse_mem_usage_kb("0 K"), Some(0));
        assert_eq!(parse_mem_usage_kb("N/A"), None);
    }

    #[test]
    fn parses_ipv6_only_dev_server_listener() {
        // Node 17+/Vite bind localhost to ::1 only. With plain `netstat -ano`
        // (no `-p TCP`), the IPv6 row reaches the parser labelled `TCP`.
        assert_eq!(
            parse_netstat_listen_line("  TCP    [::1]:5173   [::]:0   LISTENING   4242"),
            Some((5173, 4242))
        );
    }

    #[test]
    fn command_line_passes_simple_tokens_unquoted() {
        let tokens = ["npm", "run", "dev", "--", "--host", "127.0.0.1"]
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>();
        assert_eq!(windows_command_line(&tokens), "npm run dev -- --host 127.0.0.1");
    }

    #[test]
    fn command_line_quotes_spaces_and_doubles_inner_quotes() {
        let tokens = vec![
            "node".to_string(),
            "my script.js".to_string(),
            "say \"hi\"".to_string(),
        ];
        // spaces → wrapped; inner quote → doubled ("") per cmd.exe convention.
        assert_eq!(
            windows_command_line(&tokens),
            "node \"my script.js\" \"say \"\"hi\"\"\""
        );
    }
}
