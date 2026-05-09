use crate::models::{ApiError, HealthCheck, HealthStatus};
use std::time::Duration;
use tokio::{net::TcpStream, process::Command, time::timeout};

pub async fn run_health_check(
    check: &HealthCheck,
    default_cwd: Option<&str>,
) -> Result<HealthStatus, ApiError> {
    match check {
        HealthCheck::None => Ok(HealthStatus::Unknown),
        HealthCheck::Tcp {
            host,
            port,
            timeout_ms,
        } => {
            let address = format!("{host}:{port}");
            match timeout(
                Duration::from_millis(*timeout_ms),
                TcpStream::connect(address),
            )
            .await
            {
                Ok(Ok(_)) => Ok(HealthStatus::Healthy),
                Ok(Err(error)) => Err(ApiError::with_details(
                    "HEALTHCHECK_FAILED",
                    "TCP health check failed",
                    error,
                    true,
                )),
                Err(_) => Err(ApiError::new(
                    "HEALTHCHECK_FAILED",
                    "TCP health check timed out",
                    true,
                )),
            }
        }
        HealthCheck::Http {
            url,
            expected_status,
            timeout_ms,
            ..
        } => {
            let status = http_status(url, *timeout_ms).await?;
            if status == *expected_status {
                Ok(HealthStatus::Healthy)
            } else {
                Err(ApiError::with_details(
                    "HEALTHCHECK_FAILED",
                    "HTTP health check returned an unexpected status",
                    format!("expected {expected_status}, got {status}"),
                    true,
                ))
            }
        }
        HealthCheck::Custom {
            command,
            args,
            working_directory,
            timeout_ms,
        } => {
            let mut cmd = Command::new(command);
            cmd.args(args);
            if let Some(cwd) = working_directory.as_deref().or(default_cwd) {
                cmd.current_dir(cwd);
            }
            let timeout_duration = Duration::from_millis(timeout_ms.unwrap_or(12_000));
            let output = timeout(timeout_duration, cmd.output())
                .await
                .map_err(|_| {
                    ApiError::new("HEALTHCHECK_FAILED", "Custom health check timed out", true)
                })?
                .map_err(|error| {
                    ApiError::with_details(
                        "HEALTHCHECK_FAILED",
                        "Custom health check failed to execute",
                        error,
                        true,
                    )
                })?;
            if output.status.success() {
                Ok(HealthStatus::Healthy)
            } else {
                Err(ApiError::with_details(
                    "HEALTHCHECK_FAILED",
                    "Custom health check exited unsuccessfully",
                    String::from_utf8_lossy(&output.stderr),
                    true,
                ))
            }
        }
    }
}

async fn http_status(url: &str, timeout_ms: u64) -> Result<u16, ApiError> {
    let stripped = url.strip_prefix("http://").ok_or_else(|| {
        ApiError::new(
            "HEALTHCHECK_FAILED",
            "Only http:// health checks are supported in MVP",
            false,
        )
    })?;
    let (host_port, path) = stripped.split_once('/').unwrap_or((stripped, ""));
    let (host, port) = match host_port.split_once(':') {
        Some((host, port_str)) => {
            let port = port_str.parse::<u16>().map_err(|error| {
                ApiError::with_details(
                    "HEALTHCHECK_FAILED",
                    "HTTP health check URL has an invalid port",
                    error,
                    false,
                )
            })?;
            (host, port)
        }
        None => (host_port, 80),
    };
    if host.is_empty() {
        return Err(ApiError::new(
            "HEALTHCHECK_FAILED",
            "HTTP health check URL is missing a host",
            false,
        ));
    }
    let mut stream = timeout(
        Duration::from_millis(timeout_ms),
        TcpStream::connect((host, port)),
    )
    .await
    .map_err(|_| ApiError::new("HEALTHCHECK_FAILED", "HTTP health check timed out", true))?
    .map_err(|error| {
        ApiError::with_details(
            "HEALTHCHECK_FAILED",
            "HTTP health check failed to connect",
            error,
            true,
        )
    })?;
    let request = format!("GET /{path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    stream
        .write_all(request.as_bytes())
        .await
        .map_err(|error| {
            ApiError::with_details(
                "HEALTHCHECK_FAILED",
                "HTTP health check failed to write request",
                error,
                true,
            )
        })?;
    let mut buffer = vec![0_u8; 256];
    let size = stream.read(&mut buffer).await.map_err(|error| {
        ApiError::with_details(
            "HEALTHCHECK_FAILED",
            "HTTP health check failed to read response",
            error,
            true,
        )
    })?;
    let response = String::from_utf8_lossy(&buffer[..size]);
    let status = response
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or_else(|| {
            ApiError::new(
                "HEALTHCHECK_FAILED",
                "HTTP health check returned an invalid response",
                true,
            )
        })?;
    Ok(status)
}
