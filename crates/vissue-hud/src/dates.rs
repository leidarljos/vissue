//! Turn Org timestamps into ordinary dates.

/// Replace Org active/inactive stamps in `s` with a short date (and time).
///
/// `[2026-01-14 Wed 09:12]` becomes `14 Jan 2026 09:12`. A range
/// `[a]--[b]` becomes `14 Jan 2026 09:12 to 14 Jan 2026 10:42`. Text
/// that is not a stamp is left alone.
pub fn format_org_stamps(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some((i, close)) = next_stamp(rest) {
        out.push_str(&rest[..i]);
        let inner_at = i + 1;
        match rest[inner_at..].find(close) {
            Some(end) => {
                let inner = &rest[inner_at..inner_at + end];
                match format_stamp_inner(inner) {
                    Some(pretty) => {
                        out.push_str(&pretty);
                        rest = &rest[inner_at + end + 1..];
                        if rest.starts_with("--") && next_stamp(rest) == Some((2, ']'))
                            || rest.starts_with("--") && next_stamp(rest) == Some((2, '>'))
                        {
                            out.push_str(" to ");
                            rest = &rest[2..];
                        }
                    }
                    None => {
                        out.push_str(&rest[i..=inner_at + end]);
                        rest = &rest[inner_at + end + 1..];
                    }
                }
            }
            None => {
                out.push_str(rest);
                return out;
            }
        }
    }
    out.push_str(rest);
    out
}

fn next_stamp(s: &str) -> Option<(usize, char)> {
    let open_sq = s.find('[');
    let open_an = s.find('<');
    match (open_sq, open_an) {
        (Some(a), Some(b)) if a <= b => Some((a, ']')),
        (Some(a), None) => Some((a, ']')),
        (None, Some(b)) => Some((b, '>')),
        _ => None,
    }
}

fn format_stamp_inner(inner: &str) -> Option<String> {
    let mut parts = inner.split_whitespace();
    let date = parts.next()?;
    let mut bits = date.split('-');
    let year: i32 = bits.next()?.parse().ok()?;
    let month: u32 = bits.next()?.parse().ok()?;
    let day: u32 = bits.next()?.parse().ok()?;
    if bits.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let mut pretty = format!("{} {} {}", day, MONTHS[(month - 1) as usize], year);
    let mut next = parts.next();
    if next.is_some_and(is_weekday) {
        next = parts.next();
    }
    if let Some(time) = next
        && time.contains(':')
    {
        pretty.push(' ');
        pretty.push_str(time);
    }
    Some(pretty)
}

fn is_weekday(s: &str) -> bool {
    matches!(s, "Mon" | "Tue" | "Wed" | "Thu" | "Fri" | "Sat" | "Sun")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inactive_date_drops_brackets_and_weekday() {
        assert_eq!(format_org_stamps("[2026-01-12 Mon]"), "12 Jan 2026");
    }

    #[test]
    fn active_deadline_is_a_plain_date() {
        assert_eq!(format_org_stamps("<2026-03-01 Sun>"), "1 Mar 2026");
    }

    #[test]
    fn stamp_with_time_keeps_the_clock() {
        assert_eq!(
            format_org_stamps("[2026-01-14 Wed 09:12]"),
            "14 Jan 2026 09:12"
        );
    }

    #[test]
    fn clock_range_uses_to() {
        assert_eq!(
            format_org_stamps("[2026-01-14 Wed 09:12]--[2026-01-14 Wed 10:42]"),
            "14 Jan 2026 09:12 to 14 Jan 2026 10:42"
        );
    }

    #[test]
    fn non_stamp_text_is_unchanged() {
        assert_eq!(format_org_stamps("feature"), "feature");
    }

    #[test]
    fn double_dash_outside_a_stamp_range_is_left_alone() {
        assert_eq!(format_org_stamps("docs--retry"), "docs--retry");
    }
}
