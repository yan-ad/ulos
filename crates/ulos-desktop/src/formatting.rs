use ulos_core::DownloadProgress;

pub fn duration(value: std::time::Duration) -> String {
    let seconds = value.as_secs();
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 3600 {
        format!("{}m {}s", seconds / 60, seconds % 60)
    } else {
        format!("{}h {}m", seconds / 3600, seconds / 60 % 60)
    }
}

pub fn bytes(value: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut amount = value as f64;
    let mut unit = 0;
    while amount >= 1024.0 && unit + 1 < UNITS.len() {
        amount /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{value} B")
    } else {
        format!("{amount:.1} {}", UNITS[unit])
    }
}

pub fn progress_text(progress: DownloadProgress) -> String {
    let downloaded = bytes(progress.downloaded_bytes);
    match progress.total_bytes {
        Some(total) => format!("{downloaded} of {}", bytes(total)),
        None => format!("{downloaded} downloaded · total size unknown"),
    }
}

pub fn percentage(progress: DownloadProgress) -> Option<f32> {
    progress.total_bytes.map(|total| {
        if total == 0 {
            100.0
        } else {
            (progress.downloaded_bytes as f64 / total as f64 * 100.0).clamp(0.0, 100.0) as f32
        }
    })
}

pub fn decimal_bytes(value: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut amount = value as f64;
    let mut unit = 0;
    while amount >= 1000.0 && unit + 1 < UNITS.len() {
        amount /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{value} B")
    } else {
        format!("{amount:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_uses_decimal_units_through_terabytes() {
        for (bytes, expected) in [
            (0, "0 B"),
            (999, "999 B"),
            (1000, "1.0 KB"),
            (1_500_000, "1.5 MB"),
            (2_000_000_000, "2.0 GB"),
            (3_000_000_000_000, "3.0 TB"),
        ] {
            assert_eq!(decimal_bytes(bytes), expected);
        }
    }

    #[test]
    fn formats_bytes_without_inventing_decimal_units() {
        assert_eq!(bytes(0), "0 B");
        assert_eq!(bytes(1023), "1023 B");
        assert_eq!(bytes(1024), "1.0 KiB");
        assert_eq!(bytes(5 * 1024 * 1024), "5.0 MiB");
    }

    #[test]
    fn percentage_handles_empty_unknown_and_large_transfers() {
        assert_eq!(
            percentage(DownloadProgress {
                downloaded_bytes: 0,
                total_bytes: Some(0)
            }),
            Some(100.0)
        );
        assert_eq!(
            percentage(DownloadProgress {
                downloaded_bytes: 5,
                total_bytes: None
            }),
            None
        );
        assert_eq!(
            percentage(DownloadProgress {
                downloaded_bytes: 5,
                total_bytes: Some(10)
            }),
            Some(50.0)
        );
        assert_eq!(
            percentage(DownloadProgress {
                downloaded_bytes: u64::MAX,
                total_bytes: Some(u64::MAX)
            }),
            Some(100.0)
        );
    }

    #[test]
    fn unknown_length_does_not_claim_a_total() {
        assert_eq!(
            progress_text(DownloadProgress {
                downloaded_bytes: 100,
                total_bytes: None
            }),
            "100 B downloaded · total size unknown"
        );
    }
}
