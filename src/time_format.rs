pub fn format_duration(total_seconds: i64) -> String {
    if total_seconds <= 0 {
        return "0s".to_string();
    }

    let days = total_seconds / 86400;
    let hours = (total_seconds % 86400) / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;

    let mut parts = Vec::new();
    if days > 0 {
        parts.push(format!("{}d", days));
    }
    if hours > 0 || days > 0 {
        parts.push(format!("{}h", hours));
    }
    if minutes > 0 || hours > 0 || days > 0 {
        parts.push(format!("{}m", minutes));
    }
    parts.push(format!("{}s", seconds));
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
    fn keeps_zero_middle_parts_once_larger_units_are_present() {
        assert_eq!(format_duration(3601), "1h 0m 1s");
    }
}
