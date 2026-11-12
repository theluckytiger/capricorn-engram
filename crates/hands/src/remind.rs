//! `remind`: reminders and timers ("remind me in 20 minutes to check the oven", "a timer for 5 minutes", "what are my
//! reminders?"). Kept by her core (`capri_core::reminders`); the app says each one when it's due.

use async_trait::async_trait;
use chrono::{Datelike, Duration, Local, NaiveDate, NaiveTime, TimeZone, Weekday};
use serde_json::{Value, json};

use crate::tool::{Ctx, Outcome, Risk, Tool, ToolSpec, arg, opt_arg};

/// The day a reminder is for: a date ("2026-11-20", "20 november", "nov 20", "the 15th") or a day of the week
/// ("friday", "next friday": the next one after today).
fn day_of(date: &str, today: NaiveDate) -> Result<NaiveDate, String> {
    let d = date.trim().to_lowercase().replace(',', " ");
    if let Ok(day) = NaiveDate::parse_from_str(d.trim(), "%Y-%m-%d") {
        return Ok(day);
    }
    let words: Vec<&str> = d.split_whitespace().filter(|w| !["on", "the", "of", "this", "next", "coming"].contains(w)).collect();
    match words.as_slice() {
        ["today"] => return Ok(today),
        ["tomorrow"] => return today.succ_opt().ok_or_else(|| "no tomorrow".into()),
        [w] if w.parse::<Weekday>().is_ok() => {
            let want = w.parse::<Weekday>().unwrap();
            let ahead = (want.num_days_from_monday() + 7 - today.weekday().num_days_from_monday()) % 7;
            return Ok(today + Duration::days(if ahead == 0 { 7 } else { ahead as i64 }));
        }
        _ => {}
    }
    // "20 november", "nov 20", "the 15th": this year (or month), else the next.
    let number = |w: &str| w.trim_end_matches(|c: char| c.is_alphabetic()).parse::<u32>().ok().filter(|_| w.chars().next().is_some_and(|c| c.is_ascii_digit()));
    let month = |w: &str| (1..=12u32).find(|m| NaiveDate::from_ymd_opt(2000, *m, 1).is_some_and(|d| d.format("%B").to_string().to_lowercase().starts_with(&w.chars().take(3).collect::<String>()) && w.chars().count() >= 3));
    let (mut day, mut mon) = (None, None);
    for w in &words {
        if let Some(n) = number(w) {
            day = Some(n);
        } else if let Some(m) = month(w) {
            mon = Some(m);
        }
    }
    let day = day.ok_or_else(|| format!("\"{date}\" isn't a date like 2026-11-20 or a day like friday"))?;
    let pick = |y: i32, m: u32| NaiveDate::from_ymd_opt(y, m, day);
    let found = match mon {
        Some(m) => pick(today.year(), m).filter(|d| *d >= today).or_else(|| pick(today.year() + 1, m)),
        None => pick(today.year(), today.month()).filter(|d| *d >= today).or_else(|| {
            let next = today.with_day(1)? + Duration::days(32);
            pick(next.year(), next.month())
        }),
    };
    found.ok_or_else(|| format!("\"{date}\" isn't a day of the calendar"))
}

/// When, from minutes from now, or a clock time ("18:30", "6:30 pm"; past today means tomorrow, or `tomorrow`), on
/// another day with `date` (9:00 when no time is given).
pub fn when(in_minutes: Option<f64>, at: Option<&str>, date: Option<&str>, tomorrow: bool, now: chrono::DateTime<Local>) -> Result<chrono::DateTime<Local>, String> {
    if let Some(m) = in_minutes {
        if m <= 0.0 || m > 60.0 * 24.0 * 366.0 {
            return Err(format!("{m} minutes isn't a time to wait"));
        }
        return Ok(now + Duration::seconds((m * 60.0).round() as i64));
    }
    if let Some(date) = date.filter(|d| !d.trim().is_empty()) {
        let day = day_of(date, now.date_naive())?;
        let t = match at {
            Some(at) => clock(at)?,
            None => NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
        };
        let due = Local.from_local_datetime(&day.and_time(t)).single().ok_or("that time doesn't exist that day (a clock change)")?;
        if due <= now {
            return Err(format!("{} is already past", due.format("%A %-d %B %H:%M")));
        }
        return Ok(due);
    }
    let t = clock(at.ok_or("give in_minutes, at or date")?)?;
    let mut day = now.date_naive();
    if tomorrow || now.time() >= t {
        day = day.succ_opt().ok_or("no tomorrow")?;
    }
    Local.from_local_datetime(&day.and_time(t)).single().ok_or_else(|| "that time doesn't exist today (a clock change)".to_string())
}

/// A clock time as said: "18:30", "6:30 pm", "9 am".
fn clock(at: &str) -> Result<NaiveTime, String> {
    let at = at.trim().to_lowercase();
    let (clock, pm, am) = (at.trim_end_matches("pm").trim_end_matches("am").trim().to_string(), at.ends_with("pm"), at.ends_with("am"));
    let t = NaiveTime::parse_from_str(&clock, "%H:%M").or_else(|_| NaiveTime::parse_from_str(&format!("{clock}:00"), "%H:%M")).map_err(|_| format!("\"{at}\" isn't a time like 18:30"))?;
    let t = match (pm, am) {
        (true, _) if t.format("%H").to_string() != "12" => t + Duration::hours(12),
        (_, true) if t.format("%H").to_string() == "12" => t - Duration::hours(12),
        _ => t,
    };
    Ok(t)
}

pub struct Remind;

#[async_trait]
impl Tool for Remind {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "remind".into(),
            description: "Reminders and timers. `add`: `text` (what to remind them of; empty for a plain timer) and either \
                          `in_minutes` (\"in an hour and a half\" = 90) or `at` (\"18:30\", \"6:30 pm\"; add `tomorrow` for \
                          tomorrow), and `date` for another day (\"2026-11-20\", \"friday\", \"20 november\"; 9:00 without \
                          `at`). `list`: what's set. `cancel`: by `id`, or all of them. She says each one out loud \
                          when it's due."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["add", "list", "cancel"] },
                    "text": { "type": "string" },
                    "in_minutes": { "type": "number" },
                    "at": { "type": "string" },
                    "tomorrow": { "type": "boolean" },
                    "date": { "type": "string" },
                    "id": { "type": "integer" }
                },
                "required": ["action"]
            }),
        }
    }

    fn risk(&self, args: &Value) -> Risk {
        if args.get("action").and_then(Value::as_str) == Some("list") { Risk::Read } else { Risk::Normal }
    }

    async fn call(&self, args: Value, ctx: &Ctx) -> Outcome {
        let action: String = match arg(&args, "action") {
            Ok(a) => a,
            Err(o) => return o,
        };
        let now = Local::now();
        match action.as_str() {
            "add" => {
                let due = match when(opt_arg(&args, "in_minutes"), opt_arg::<String>(&args, "at").as_deref(), opt_arg::<String>(&args, "date").as_deref(), opt_arg(&args, "tomorrow").unwrap_or(false), now) {
                    Ok(d) => d,
                    Err(e) => return Outcome::failed(e),
                };
                let text: String = opt_arg(&args, "text").unwrap_or_default();
                let what = if text.trim().is_empty() { "a timer".to_string() } else { format!("\"{}\"", text.trim()) };
                let at = due.format("%H:%M");
                let day = match (due.date_naive() - now.date_naive()).num_days() {
                    0 => "today".to_string(),
                    1 => "tomorrow".to_string(),
                    2..=6 => due.format("%A").to_string(),
                    _ => due.format("%A %-d %B").to_string(),
                };
                if ctx.dry_run {
                    return Outcome::ok(format!("(dry run) would set {what} for {at} {day}"));
                }
                match capri_core::reminders::add(due, &text) {
                    Ok(r) => Outcome::ok(format!("set {what} for {at} {day} (in {} min; id {})", (due - now).num_minutes().max(1), r.id)),
                    Err(e) => Outcome::failed(format!("couldn't save it: {e:#}")),
                }
            }
            "list" => {
                let all = capri_core::reminders::all();
                if all.is_empty() {
                    return Outcome::ok("no reminders or timers set");
                }
                let lines: Vec<String> = all.iter().map(|r| format!("id {}: {} at {}", r.id, if r.text.is_empty() { "timer" } else { &r.text }, r.due.format(if (r.due.date_naive() - now.date_naive()).num_days() > 6 { "%a %-d %b %H:%M" } else { "%a %H:%M" }))).collect();
                Outcome::ok(lines.join("; "))
            }
            "cancel" => {
                let id: Option<u64> = opt_arg(&args, "id");
                if ctx.dry_run {
                    return Outcome::ok(format!("(dry run) would cancel {}", id.map_or("all of them".into(), |i| format!("id {i}"))));
                }
                match capri_core::reminders::remove(|r| id.is_some_and(|i| r.id != i)) {
                    Ok(gone) if gone.is_empty() => Outcome::failed("nothing like that was set"),
                    Ok(gone) => Outcome::ok(format!("cancelled {}", gone.iter().map(|r| if r.text.is_empty() { "a timer".to_string() } else { format!("\"{}\"", r.text) }).collect::<Vec<_>>().join(", "))),
                    Err(e) => Outcome::failed(format!("{e:#}")),
                }
            }
            other => Outcome::failed(format!("unknown action \"{other}\"")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_as_said() {
        let now = Local.with_ymd_and_hms(2026, 11, 3, 17, 0, 0).unwrap();
        assert_eq!(when(Some(90.0), None, None, false, now).unwrap(), now + Duration::minutes(90));
        assert_eq!(when(None, Some("18:30"), None, false, now).unwrap().format("%d %H:%M").to_string(), "03 18:30");
        assert_eq!(when(None, Some("6:30 pm"), None, false, now).unwrap().format("%d %H:%M").to_string(), "03 18:30");
        // Already past today: tomorrow.
        assert_eq!(when(None, Some("9:00"), None, false, now).unwrap().format("%d %H:%M").to_string(), "04 09:00");
        assert_eq!(when(None, Some("12 am"), None, true, now).unwrap().format("%d %H:%M").to_string(), "04 00:00");
        assert!(when(Some(-5.0), None, None, false, now).is_err());
        assert!(when(None, Some("teatime"), None, false, now).is_err());
    }

    #[test]
    fn days_as_said() {
        // A Tuesday.
        let now = Local.with_ymd_and_hms(2026, 11, 3, 17, 0, 0).unwrap();
        let on = |date: &str, at: Option<&str>| when(None, at, Some(date), false, now).map(|d| d.format("%Y-%m-%d %H:%M").to_string());
        assert_eq!(on("2026-11-20", Some("18:30")).unwrap(), "2026-11-20 18:30");
        assert_eq!(on("friday", None).unwrap(), "2026-11-06 09:00");
        assert_eq!(on("next tuesday", Some("8 am")).unwrap(), "2026-11-10 08:00");
        assert_eq!(on("20 November", None).unwrap(), "2026-11-20 09:00");
        assert_eq!(on("jan 5", None).unwrap(), "2027-01-05 09:00");
        assert_eq!(on("the 15th", None).unwrap(), "2026-11-15 09:00");
        assert_eq!(on("the 2nd", None).unwrap(), "2026-12-02 09:00");
        assert_eq!(on("tomorrow", Some("7:00")).unwrap(), "2026-11-04 07:00");
        assert!(on("2026-11-01", None).is_err());
        assert!(on("someday", None).is_err());
    }
}
