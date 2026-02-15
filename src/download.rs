use std::fs;
use std::io::{Read, Write};
use std::path::Path;
use std::thread;
use std::time::Duration;

use crate::logger::Logger;

//
// ===================== Configuration =====================
//

/// Network connect/read timeout per attempt.
const TIMEOUT: Duration = Duration::from_secs(30);

/// Max sleep seconds between retries (backoff cap).
/// - For network/transport errors: reaching this cap still allows further retries (sleep stays capped).
/// - For HTTP status errors: once backoff reaches/exceeds this cap, retries stop (as requested).
const MAX_RETRY_SLEEP_SECS: u64 = 60;

//
// ===================== Implementation =====================
//

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum RetryKind {
    Network,   // ureq::Error::Transport(...)
    HttpStatus, // ureq::Error::Status(...)
    Io,        // read/write filesystem errors
}

#[inline]
fn retryable_http_status(code: u16) -> bool {
    // Success-rate oriented:
    // - 5xx: server side transient
    // - 429: rate limit
    // - 408: request timeout
    // - 425: too early (rare, but transient)
    matches!(code, 408 | 425 | 429) || code >= 500
}

#[inline]
fn retryable_io_error(kind: std::io::ErrorKind) -> bool {
    // Success-rate oriented (some of these may be transient on Windows due to locks/AV):
    matches!(
        kind,
        std::io::ErrorKind::Interrupted
            | std::io::ErrorKind::WouldBlock
            | std::io::ErrorKind::TimedOut
            | std::io::ErrorKind::PermissionDenied
            | std::io::ErrorKind::Other
    )
}

#[inline]
fn compute_backoff_secs(base_delay: u64, attempt: u32) -> u64 {
    // Exponential backoff: base * 2^attempt, saturating.
    // Avoid shift overflow by clamping shift amount.
    let shift = attempt.min(62); // 2^62 is already huge; keep safe
    let exp = 1u64.checked_shl(shift).unwrap_or(u64::MAX);
    base_delay.saturating_mul(exp)
}

/// Download a file from `url` to `path` with retry logic.
pub fn download_file(
    url: &str,
    path: &Path,
    logger: &mut Logger,
    retry_delay: u32,
    retry_count: u32,
) -> bool {
    if retry_count == 0 {
        logger.log(&format!("retry_count=0, refusing to download {url}"));
        return false;
    }

    let agent = ureq::AgentBuilder::new()
        .timeout_connect(TIMEOUT)
        .timeout_read(TIMEOUT)
        .build();

    let base_delay = retry_delay.max(1) as u64;

    for attempt in 0..retry_count {
        let attempt_no = attempt + 1;

        // Execute one attempt
        let outcome: Result<(), (RetryKind, String, bool)> = match agent.get(url).call() {
            Ok(resp) => {
                // 2xx only (ureq treats non-2xx as Err(Status))
                let mut buf = Vec::new();

                match resp.into_reader().read_to_end(&mut buf) {
                    Ok(_) => {
                        // Write atomically-ish: create parent dirs if missing? (caller usually ensures)
                        // Use a temp file then rename to reduce partial writes on crash.
                        let tmp_path = path.with_extension("tmp");
                        match fs::File::create(&tmp_path) {
                            Ok(mut f) => {
                                if let Err(e) = f.write_all(&buf) {
                                    let retry = retryable_io_error(e.kind());
                                    Err((
                                        RetryKind::Io,
                                        format!("Failed to write temp file for {url}: {e}"),
                                        retry,
                                    ))
                                } else if let Err(e) = f.flush() {
                                    let retry = retryable_io_error(e.kind());
                                    Err((
                                        RetryKind::Io,
                                        format!("Failed to flush temp file for {url}: {e}"),
                                        retry,
                                    ))
                                } else if let Err(e) = fs::rename(&tmp_path, path) {
                                    // On Windows rename may fail if target exists; try remove then rename.
                                    let retry = retryable_io_error(e.kind());
                                    if path.exists() {
                                        let _ = fs::remove_file(path);
                                    }
                                    match fs::rename(&tmp_path, path) {
                                        Ok(_) => Ok(()),
                                        Err(e2) => Err((
                                            RetryKind::Io,
                                            format!("Failed to move temp file into place for {url}: {e2}"),
                                            retryable_io_error(e2.kind()) || retry,
                                        )),
                                    }
                                } else {
                                    Ok(())
                                }
                            }
                            Err(e) => {
                                let retry = retryable_io_error(e.kind());
                                Err((
                                    RetryKind::Io,
                                    format!("Failed to create temp file for {url}: {e}"),
                                    retry,
                                ))
                            }
                        }
                    }
                    Err(e) => {
                        // Treat read errors as transient
                        Err((
                            RetryKind::Io,
                            format!(
                                "Failed to read response for {url} (attempt {attempt_no}/{retry_count}): {e}"
                            ),
                            true,
                        ))
                    }
                }
            }
            Err(e) => match e {
                ureq::Error::Status(code, _resp) => {
                    let code_u16 = code as u16;
                    if retryable_http_status(code_u16) {
                        Err((
                            RetryKind::HttpStatus,
                            format!(
                                "Server returned status {code_u16} for {url} (attempt {attempt_no}/{retry_count}), will retry"
                            ),
                            true,
                        ))
                    } else {
                        Err((
                            RetryKind::HttpStatus,
                            format!(
                                "Non-retryable HTTP status {code_u16} for {url} (attempt {attempt_no}/{retry_count}), aborting"
                            ),
                            false,
                        ))
                    }
                }
                ureq::Error::Transport(err) => Err((
                    RetryKind::Network,
                    format!(
                        "Transport error downloading {url} (attempt {attempt_no}/{retry_count}): {err}"
                    ),
                    true,
                )),
            },
        };

        match outcome {
            Ok(()) => {
                logger.log(&format!("Downloaded {url}"));
                return true;
            }
            Err((kind, msg, should_retry)) => {
                logger.log(&msg);

                if !should_retry {
                    return false;
                }

                if attempt_no >= retry_count {
                    break;
                }

                // Compute backoff
                let backoff = compute_backoff_secs(base_delay, attempt as u32);
                let capped = backoff.min(MAX_RETRY_SLEEP_SECS);

                // As requested:
                // - If backoff reaches/exceeds cap:
                //   - Network errors: continue retrying (sleep stays capped).
                //   - HTTP status errors: stop retrying once cap is reached/exceeded.
                if backoff >= MAX_RETRY_SLEEP_SECS && kind == RetryKind::HttpStatus {
                    logger.log(&format!(
                        "Backoff reached cap ({}s) for HTTP status retries of {url}; stopping retries as configured",
                        MAX_RETRY_SLEEP_SECS
                    ));
                    return false;
                }

                logger.log(&format!(
                    "Waiting {}s before next attempt for {url} (attempt {}/{})",
                    capped,
                    attempt_no,
                    retry_count
                ));
                thread::sleep(Duration::from_secs(capped));
            }
        }
    }

    logger.log(&format!(
        "Failed to download {url} after {retry_count} attempts"
    ));
    false
}
