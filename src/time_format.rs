/// Formats a duration in seconds into a human-readable string like "1h 5m 10s".
///
/// Omits zero-value components for brevity, e.g., `3601` seconds becomes "1h 1s".
pub fn format_duration(total_seconds: i64) -> String {
    // Handle non-positive durations as a special case.
    if total_seconds <= 0 {
        return "0s".to_string();
    }

    // Calculate each time component.
    let days = total_seconds / 86400;
    let hours = (total_seconds % 86400) / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;

    let mut parts = Vec::new();

    // Build the string part by part, only including non-zero values.
    if days > 0 {
        parts.push(format!("{}d", days));
    }
    if hours > 0 {
        parts.push(format!("{}h", hours));
    }
    if minutes > 0 {
        parts.push(format!("{}m", minutes));
    }
    // Always include seconds if it's non-zero, or if it's the only unit
    // (e.g., for a duration of less than 1 minute).
    if seconds > 0 || parts.is_empty() {
        parts.push(format!("{}s", seconds));
    }

    parts.join(" ")
}

#[cfg(test)]
mod tests {
    use super::format_duration;

    #[test]
    fn formats_zero_and_negative_as_zero_seconds() {
        assert_eq!(format_duration(0), "0s");
        assert_eq!(format_duration(-10), "0s");
    }

    #[test]
    fn formats_duration_with_all_parts() {
        assert_eq!(format_duration(90061), "1d 1h 1m 1s");
    }

    #[test]
    fn omits_zero_middle_parts() {
        assert_eq!(format_duration(3601), "1h 1s");
    }

    #[test]
    fn omits_smaller_zero_parts() {
        assert_eq!(format_duration(3600), "1h");
    }
}
