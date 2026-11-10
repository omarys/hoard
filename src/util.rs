use std::time::{SystemTime, UNIX_EPOCH};

/// `YYYY-MM-DD` for a timestamp, or `None` when it predates the Unix epoch.
pub fn iso_date(time: SystemTime) -> Option<String> {
    let millis = time.duration_since(UNIX_EPOCH).ok()?.as_millis();
    let days = i64::try_from(millis / 1_000).ok()? / 86_400;
    let (year, month, day) = civil_from_days(days);
    Some(format!("{year:04}-{month:02}-{day:02}"))
}

/// Howard Hinnant's `civil_from_days`, for the days since 1970-01-01.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let mp = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (
        if month <= 2 { year + 1 } else { year },
        u32::try_from(month).unwrap_or(1),
        u32::try_from(day).unwrap_or(1),
    )
}

pub fn translate_number_to_nth(count: u16) -> String {
    match count {
        0 => "first".to_string(),
        1 => "second".to_string(),
        2 => "third".to_string(),
        3 => "fourth".to_string(),
        4 => "fifth".to_string(),
        5 => "sixth".to_string(),
        6 => "seventh".to_string(),
        7 => "eighth".to_string(),
        _ => "nth".to_string(),
    }
}

pub fn string_find_next(s: &str, from: &str, to: &str) -> String {
    // Find substring of a string `s` from first occurrence of `from` to first occurrence of `to` after `from` was encountered
    // For example:
    // `s` = "Hello #my test" | `from` = "#" | `to` = " "
    // Returns "#my"
    if s.contains(from) {
        let split = s.split(from).nth(1).unwrap().split(to).next().unwrap();
        return format!("{from}{split}");
    }
    String::new()
}

pub fn split_with_delim(s: &str, delim: &str) -> Vec<String> {
    //Credits to chatGPT for simplifying this wtf
    let mut result = Vec::new();
    let mut start = 0;

    for (i, _) in s.match_indices(delim) {
        if i > start {
            result.push(s[start..i].to_string());
        }
        result.push(delim.to_string());
        start = i + delim.len();
    }

    if start < s.len() {
        result.push(s[start..].to_string());
    }

    while result.first().is_some_and(String::is_empty) {
        result.remove(0);
    }

    while result.last().is_some_and(String::is_empty) {
        result.pop();
    }

    result
}

#[cfg(test)]
mod test_commands {
    use super::*;

    #[test]
    fn iso_date_formats_known_days() {
        let day = |days: u64| UNIX_EPOCH + std::time::Duration::from_secs(days * 86_400);
        assert_eq!(iso_date(day(0)), Some("1970-01-01".to_string()));
        assert_eq!(iso_date(day(20_512)), Some("2026-02-28".to_string()));
        assert_eq!(
            iso_date(UNIX_EPOCH - std::time::Duration::from_secs(1)),
            None
        );
    }

    #[test]
    fn extract_named_parameter() {
        let s = String::from("Hello #test command");
        let expected = "#test";
        assert_eq!(expected, string_find_next(&s, "#", " "));
    }

    #[test]
    fn extract_named_parameter_first() {
        let s = String::from("#test command");
        let expected = "#test";
        assert_eq!(expected, string_find_next(&s, "#", " "));
    }

    #[test]
    fn extract_named_parameter_last() {
        let s = String::from("command #test");
        let expected = "#test";
        assert_eq!(expected, string_find_next(&s, "#", " "));
    }

    #[test]
    fn extract_named_parameter_noname() {
        let s = String::from("test # bar");
        let expected = "#";
        assert_eq!(expected, string_find_next(&s, "#", " "));
    }

    #[test]
    fn extract_named_parameter_none() {
        let s = String::from("command test");
        let expected = "";
        assert_eq!(expected, string_find_next(&s, "#", " "));
    }

    #[test]
    fn test_split_with_delim() {
        let s = String::from("command #param test #param lol");
        let expected = vec!["command ", "#param", " test ", "#param", " lol"];
        assert_eq!(expected, split_with_delim(&s, "#param"));
    }

    #[test]
    fn test_split_with_delim_at_end() {
        let s = String::from("command #param test #param lol #param");
        let expected = vec!["command ", "#param", " test ", "#param", " lol ", "#param"];
        assert_eq!(expected, split_with_delim(&s, "#param"));
    }

    #[test]
    fn test_split_with_delim_at_start() {
        let s = String::from("#param test #param lol");
        let expected = vec!["#param", " test ", "#param", " lol"];
        assert_eq!(expected, split_with_delim(&s, "#param"));
    }
}
