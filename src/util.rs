use std::path::{Path, PathBuf};

const UNITS: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];

pub fn format_bytes(bytes: u64) -> String {
    if bytes < 1000 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit = 0usize;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    format!("{value:.2} {}", UNITS[unit])
}

pub fn format_speed(bytes_per_sec: f64) -> String {
    if bytes_per_sec <= 0.0 {
        return "--".to_string();
    }
    format!("{}/s", format_bytes(bytes_per_sec as u64))
}

pub fn format_eta(remaining: u64, bytes_per_sec: f64) -> String {
    if bytes_per_sec <= 0.0 {
        return "--".to_string();
    }
    format_secs(remaining as f64 / bytes_per_sec)
}

pub fn format_secs(secs: f64) -> String {
    if !secs.is_finite() || secs < 0.0 {
        return "--".to_string();
    }
    let total = secs.round() as u64;
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    if h > 0 {
        format!("{h}h {m:02}m")
    } else if m > 0 {
        format!("{m}m {s:02}s")
    } else {
        format!("{s}s")
    }
}

pub fn format_elapsed(secs: f64) -> String {
    format_secs(secs)
}

pub fn expand_tilde(input: &str) -> PathBuf {
    if input == "~"
        && let Some(home) = dirs::home_dir()
    {
        return home;
    }
    if let Some(rest) = input.strip_prefix("~/")
        && let Some(home) = dirs::home_dir()
    {
        return home.join(rest);
    }
    PathBuf::from(input)
}

/// Shorten a path from the middle so it fits in `width` display columns.
pub fn truncate_middle(input: &str, width: usize) -> String {
    let chars: Vec<char> = input.chars().collect();
    if chars.len() <= width || width < 4 {
        return input.to_string();
    }
    let keep = width - 1;
    let front = keep / 2;
    let back = keep - front;
    let mut out: String = chars[..front].iter().collect();
    out.push('…');
    out.extend(chars[chars.len() - back..].iter());
    out
}

/// Truncate from the end, appending an ellipsis.
pub fn truncate_end(input: &str, width: usize) -> String {
    let chars: Vec<char> = input.chars().collect();
    if chars.len() <= width || width < 2 {
        return input.to_string();
    }
    let mut out: String = chars[..width - 1].iter().collect();
    out.push('…');
    out
}

pub fn display_path(path: &Path) -> String {
    let home = dirs::home_dir();
    if let Some(home) = home
        && let Ok(stripped) = path.strip_prefix(&home)
    {
        return format!("~/{}", stripped.display());
    }
    path.display().to_string()
}

pub fn now_iso8601() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format_epoch(secs)
}

/// Minimal UTC formatter (avoids a chrono dependency).
pub fn format_epoch(secs: u64) -> String {
    let days = secs / 86400;
    let rem = secs % 86400;
    let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let (y, mo, d) = civil_from_days(days as i64);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes() {
        assert_eq!(format_bytes(999), "999 B");
        assert_eq!(format_bytes(1500), "1.50 KB");
        assert_eq!(format_bytes(3_500_000_000), "3.50 GB");
    }

    #[test]
    fn durations() {
        assert_eq!(format_secs(45.0), "45s");
        assert_eq!(format_secs(125.0), "2m 05s");
        assert_eq!(format_secs(7325.0), "2h 02m");
    }

    #[test]
    fn truncation() {
        assert_eq!(truncate_middle("abcdefghij", 5), "ab…ij");
        assert_eq!(truncate_end("abcdefghij", 5), "abcd…");
    }
}
