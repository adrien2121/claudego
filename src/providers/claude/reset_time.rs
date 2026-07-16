use chrono::{DateTime, Datelike, Local, TimeZone};
use regex::Regex;
use std::sync::LazyLock;

static LIMIT_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"resets\s+(?:([A-Za-z]{3})\s+(\d{1,2})\s+at\s+)?(\d{1,2})(?::(\d{2}))?\s*(am|pm)")
        .unwrap()
});

pub(crate) fn parse_reset_time(
    log_time: DateTime<Local>,
    content_text: &str,
) -> Option<(DateTime<Local>, String)> {
    let caps = LIMIT_REGEX.captures(content_text)?;
    let hour: u32 = caps.get(3)?.as_str().parse().unwrap_or(0);
    let minute: u32 = caps.get(4).map_or(0, |m| m.as_str().parse().unwrap_or(0));
    let period = caps.get(5)?.as_str();

    let mut mil_hour = hour;
    if period == "pm" && hour != 12 {
        mil_hour += 12;
    } else if period == "am" && hour == 12 {
        mil_hour = 0;
    }

    let target_time = if let (Some(month_cap), Some(day_cap)) = (caps.get(1), caps.get(2)) {
        let month = parse_month(month_cap.as_str())?;
        let day = day_cap.as_str().parse::<u32>().ok()?;
        let mut target = local_datetime(log_time.year(), month, day, mil_hour, minute)?
            + chrono::Duration::seconds(5);
        if target < log_time {
            target = local_datetime(log_time.year() + 1, month, day, mil_hour, minute)?
                + chrono::Duration::seconds(5);
        }
        target
    } else {
        let mut target = local_datetime(
            log_time.year(),
            log_time.month(),
            log_time.day(),
            mil_hour,
            minute,
        )? + chrono::Duration::seconds(5);

        if target < log_time {
            target += chrono::Duration::days(1);
        }
        target
    };

    let display = if caps.get(1).is_some() {
        format!(
            "{} {} at {}{}",
            caps.get(1).unwrap().as_str(),
            caps.get(2).unwrap().as_str(),
            hour,
            period
        )
    } else {
        format!("{}:{:02}{}", hour, minute, period)
    };

    Some((target_time, display))
}

fn local_datetime(
    year: i32,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
) -> Option<DateTime<Local>> {
    let date = chrono::NaiveDate::from_ymd_opt(year, month, day)?;
    let datetime = date.and_hms_opt(hour, minute, 0)?;

    match Local.from_local_datetime(&datetime) {
        chrono::offset::LocalResult::Single(time) => Some(time),
        chrono::offset::LocalResult::Ambiguous(time1, _) => Some(time1),
        chrono::offset::LocalResult::None => {
            let shifted = datetime + chrono::Duration::hours(1);
            match Local.from_local_datetime(&shifted) {
                chrono::offset::LocalResult::Single(time) => Some(time),
                chrono::offset::LocalResult::Ambiguous(time1, _) => Some(time1),
                _ => None,
            }
        }
    }
}

fn parse_month(month: &str) -> Option<u32> {
    match month.to_lowercase().as_str() {
        "jan" => Some(1),
        "feb" => Some(2),
        "mar" => Some(3),
        "apr" => Some(4),
        "may" => Some(5),
        "jun" => Some(6),
        "jul" => Some(7),
        "aug" => Some(8),
        "sep" => Some(9),
        "oct" => Some(10),
        "nov" => Some(11),
        "dec" => Some(12),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::parse_reset_time;
    use chrono::{Local, TimeZone};

    #[test]
    fn parses_same_day_time_with_minutes() {
        let log_time = Local.with_ymd_and_hms(2026, 7, 4, 12, 0, 0).unwrap();

        let (target, display) =
            parse_reset_time(log_time, "Claude limit reached; resets 5:30pm").expect("reset time");

        assert_eq!(display, "5:30pm");
        assert_eq!(
            target,
            Local.with_ymd_and_hms(2026, 7, 4, 17, 30, 5).unwrap()
        );
    }

    #[test]
    fn rolls_time_only_resets_to_next_day_when_needed() {
        let log_time = Local.with_ymd_and_hms(2026, 7, 4, 18, 0, 0).unwrap();

        let (target, display) =
            parse_reset_time(log_time, "Claude limit reached; resets 5pm").expect("reset time");

        assert_eq!(display, "5:00pm");
        assert_eq!(
            target,
            Local.with_ymd_and_hms(2026, 7, 5, 17, 0, 5).unwrap()
        );
    }

    #[test]
    fn rolls_11pm_event_with_2am_reset_to_next_day() {
        let event = Local.with_ymd_and_hms(2026, 7, 12, 23, 0, 0).unwrap();
        let (target, _) = parse_reset_time(event, "Claude limit reached; resets 2am").unwrap();
        assert_eq!(
            target,
            Local.with_ymd_and_hms(2026, 7, 13, 2, 0, 5).unwrap()
        );
    }

    #[test]
    fn rolls_1159pm_event_with_midnight_reset_to_next_day() {
        let event = Local.with_ymd_and_hms(2026, 7, 12, 23, 59, 0).unwrap();
        let (target, _) = parse_reset_time(event, "Claude limit reached; resets 12am").unwrap();
        assert_eq!(
            target,
            Local.with_ymd_and_hms(2026, 7, 13, 0, 0, 5).unwrap()
        );
    }

    #[test]
    fn rolls_11pm_event_with_noon_reset_to_next_day() {
        let event = Local.with_ymd_and_hms(2026, 7, 12, 23, 0, 0).unwrap();
        let (target, _) = parse_reset_time(event, "Claude limit reached; resets 12pm").unwrap();
        assert_eq!(
            target,
            Local.with_ymd_and_hms(2026, 7, 13, 12, 0, 5).unwrap()
        );
    }
}
