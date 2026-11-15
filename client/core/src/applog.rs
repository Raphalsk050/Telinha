//! The app's log file. It is what is left to understand why Telinha did not open, closed by
//! itself or did not update on someone else's computer. Same file and line format as before.

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

const MAX_BYTES: u64 = 512 * 1024;

pub struct AppLog {
    file: PathBuf,
    lock: Mutex<()>,
}

impl AppLog {
    pub fn new(file: impl Into<PathBuf>) -> Self {
        Self { file: file.into(), lock: Mutex::new(()) }
    }

    pub fn file(&self) -> &std::path::Path {
        &self.file
    }

    /// Past the limit, only the newer half stays.
    fn trim(&self) {
        let Ok(size) = fs::metadata(&self.file).map(|metadata| metadata.len()) else { return };
        if size <= MAX_BYTES {
            return;
        }
        let Ok(bytes) = fs::read(&self.file) else { return };
        let half = &bytes[bytes.len() / 2..];
        let start = half.iter().position(|byte| *byte == b'\n').map_or(0, |position| position + 1);
        let _ = fs::write(&self.file, &half[start..]);
    }

    fn write(&self, level: &str, text: &str) {
        let _guard = self.lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let body = text.replace("\r\n", "\n").replace('\n', "\n    ");
        if let Some(parent) = self.file.parent() {
            // Without disk or permission the app goes on without a log.
            let _ = fs::create_dir_all(parent);
        }
        self.trim();
        if let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(&self.file) {
            let _ = writeln!(file, "{} {level} {body}", iso_now());
        }
    }

    pub fn info(&self, text: &str) {
        self.write("info", text);
    }

    pub fn error(&self, text: &str) {
        self.write("erro", text);
    }
}

/// The time in UTC as `2026-11-14T20:30:38.264Z`.
pub fn iso_now() -> String {
    iso_time(crate::text::now_ms())
}

pub fn iso_time(milliseconds: i64) -> String {
    let days = milliseconds.div_euclid(86_400_000);
    let rest = milliseconds.rem_euclid(86_400_000);
    // Civil date from a day count, by Howard Hinnant's algorithm.
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 { month_index + 3 } else { month_index - 9 };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        rest / 3_600_000,
        rest / 60_000 % 60,
        rest / 1000 % 60,
        rest % 1000
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_are_written_like_the_javascript_app_wrote_them() {
        assert_eq!(iso_time(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso_time(1_794_688_238_264), "2026-11-14T20:30:38.264Z");
        assert_eq!(iso_time(951_782_400_000), "2000-02-29T00:00:00.000Z");
    }

    #[test]
    fn lines_are_appended_and_a_long_text_is_indented() {
        let folder = tempfile::tempdir().unwrap();
        let log = AppLog::new(folder.path().join("logs").join("telinha.log"));
        log.info("abrindo");
        log.error("falhou\nna segunda linha");
        let text = fs::read_to_string(log.file()).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert!(lines[0].ends_with(" info abrindo"));
        assert!(lines[1].ends_with(" erro falhou"));
        assert_eq!(lines[2], "    na segunda linha");
    }

    #[test]
    fn a_log_past_the_limit_keeps_the_newer_half() {
        let folder = tempfile::tempdir().unwrap();
        let log = AppLog::new(folder.path().join("telinha.log"));
        let line = format!("{}\n", "x".repeat(99));
        fs::write(log.file(), line.repeat(6000)).unwrap();
        log.info("depois do corte");
        let text = fs::read_to_string(log.file()).unwrap();
        assert!(text.len() < 400 * 1024);
        assert!(text.starts_with('x') && text.ends_with("info depois do corte\n"));
    }
}
