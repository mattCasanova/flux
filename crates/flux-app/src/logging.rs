//! Rolling file logs + panic hook for crash dumps.
//!
//! Called once from `main.rs` before any other initialization. Installs
//! `flexi_logger` as the global `log` backend, rotating
//! `~/.local/state/flux/flux.log` at a 5 MB cap with 3 rotated files
//! kept. Also installs a panic hook that writes a crash report to
//! `~/.local/state/flux/crashes/{timestamp}.log` and prints a friendly
//! message pointing at the file and the issue tracker.
//!
//! Neither logs nor crash dumps leave the user's machine. Nothing in
//! this module sends data over the network — this is local diagnostics,
//! not telemetry.

use std::io::Write;
use std::panic::PanicHookInfo;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use flexi_logger::{Cleanup, Criterion, FileSpec, Logger, Naming};

use crate::platform;

/// Initialize logging and install the panic hook. Call first in `main`.
pub fn init() -> anyhow::Result<()> {
    init_file_logger()?;
    install_panic_hook();
    log::info!(
        "Flux {} ({}) starting (os={}, arch={})",
        env!("CARGO_PKG_VERSION"),
        option_env!("FLUX_GIT_SHA").unwrap_or("unknown"),
        std::env::consts::OS,
        std::env::consts::ARCH,
    );
    Ok(())
}

fn init_file_logger() -> anyhow::Result<()> {
    let log_dir = platform::state_dir();

    // Respect RUST_LOG if set, else default to info-level. Warnings and
    // errors also go to stderr so dev runs stay visible.
    let spec = std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string());

    Logger::try_with_str(&spec)?
        .log_to_file(FileSpec::default().directory(&log_dir).basename("flux"))
        .rotate(
            Criterion::Size(5 * 1024 * 1024),
            Naming::Timestamps,
            Cleanup::KeepLogFiles(3),
        )
        .append()
        .duplicate_to_stderr(flexi_logger::Duplicate::Warn)
        .format_for_files(flexi_logger::detailed_format)
        .start()?;

    Ok(())
}

/// Where to write crash reports. Cached at startup so the panic hook
/// doesn't have to resolve paths from inside a broken state.
static CRASH_DIR: OnceLock<PathBuf> = OnceLock::new();

fn install_panic_hook() {
    let _ = CRASH_DIR.set(platform::crashes_dir());

    // Preserve the default hook so debug builds still print the panic
    // + backtrace to stderr; our file write + user message run after.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        default_hook(info);
        let report = write_crash_report(info);
        print_user_message(report.as_deref());
    }));
}

fn write_crash_report(info: &PanicHookInfo<'_>) -> Option<PathBuf> {
    let dir = CRASH_DIR.get()?;
    let path = dir.join(format!("{}.log", timestamp_filename()));
    let mut file = std::fs::File::create(&path).ok()?;

    let backtrace = std::backtrace::Backtrace::force_capture();
    let location = info
        .location()
        .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
        .unwrap_or_else(|| "<unknown>".to_string());

    writeln!(
        file,
        "Flux crash report\n\
         =================\n\
         \n\
         Version:   {}\n\
         Commit:    {}\n\
         OS:        {}\n\
         Arch:      {}\n\
         Location:  {}\n\
         \n\
         Panic message:\n\
         {}\n\
         \n\
         Backtrace:\n\
         {}",
        env!("CARGO_PKG_VERSION"),
        option_env!("FLUX_GIT_SHA").unwrap_or("<unknown>"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        location,
        panic_message(info),
        backtrace,
    )
    .ok()?;

    Some(path)
}

fn print_user_message(report: Option<&std::path::Path>) {
    eprintln!();
    eprintln!("⚡ Flux crashed.");
    eprintln!();
    match report {
        Some(path) => {
            eprintln!("A crash report was saved to:");
            eprintln!("  {}", path.display());
        }
        None => {
            eprintln!("(Writing the crash report also failed.)");
        }
    }
    eprintln!();
    eprintln!("Please open an issue at:");
    eprintln!("  https://github.com/mattCasanova/flux/issues/new");
    eprintln!("and attach the crash report. Nothing was sent anywhere —");
    eprintln!("the report exists only on this machine.");
    eprintln!();
}

fn panic_message(info: &PanicHookInfo<'_>) -> String {
    let payload = info.payload();
    if let Some(s) = payload.downcast_ref::<&str>() {
        s.to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "<non-string panic payload>".to_string()
    }
}

/// yyyy-mm-ddTHH-MM-SS from SystemTime — avoids pulling in `chrono`
/// for a filename. Two crashes in the same second overwrite; fine for
/// a local report.
fn timestamp_filename() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format_timestamp(now)
}

fn format_timestamp(unix_seconds: u64) -> String {
    let days = (unix_seconds / 86_400) as i64;
    let secs = unix_seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{:04}-{:02}-{:02}T{:02}-{:02}-{:02}",
        year,
        month,
        day,
        secs / 3600,
        (secs / 60) % 60,
        secs % 60
    )
}

/// Civil date from days since the Unix epoch — Howard Hinnant's
/// algorithm (public domain).
fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    (year as i32, m as u32, d as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_epoch_zero() {
        assert_eq!(format_timestamp(0), "1970-01-01T00-00-00");
    }

    #[test]
    fn formats_known_date() {
        // 2024-01-15 12:33:16 UTC
        assert_eq!(format_timestamp(1705321996), "2024-01-15T12-33-16");
    }

    #[test]
    fn leap_year_boundary() {
        // 2024-02-29 00:00:00 UTC = 1709164800
        assert_eq!(format_timestamp(1709164800), "2024-02-29T00-00-00");
    }
}
