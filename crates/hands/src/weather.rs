//! Real weather: Open-Meteo (free, no key) for now, hour by hour and the next days, and when rain starts or stops.
//! If it's down, wttr.in; only when both fail does she say so.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::tool::{Ctx, Outcome, Risk, Tool, ToolSpec, opt_arg};

pub struct Weather {
    http: reqwest::Client,
    places: Mutex<HashMap<String, Place>>,
}

#[derive(Debug, Clone)]
struct Place {
    label: String,
    lat: f64,
    lon: f64,
}

impl Weather {
    pub fn new() -> Self {
        let http = reqwest::Client::builder().timeout(Duration::from_secs(8)).user_agent("Capricorn/0.1").build().expect("http client");
        Self { http, places: Mutex::new(HashMap::new()) }
    }

    async fn place(&self, name: &str) -> Result<Place> {
        let key = name.trim().to_lowercase();
        if let Some(p) = self.places.lock().unwrap().get(&key) {
            return Ok(p.clone());
        }
        // Open-Meteo's geocoder wants the city alone: "Lyon, France" → "Lyon", then the rest picks among results.
        let mut parts = name.split(',').map(str::trim);
        let city = parts.next().unwrap_or(name);
        let hint = parts.next().map(str::to_lowercase);
        #[derive(Deserialize)]
        struct Geo {
            results: Option<Vec<GeoResult>>,
        }
        #[derive(Deserialize)]
        struct GeoResult {
            name: String,
            latitude: f64,
            longitude: f64,
            country: Option<String>,
            admin1: Option<String>,
        }
        let geo: Geo = self
            .http
            .get("https://geocoding-api.open-meteo.com/v1/search")
            .query(&[("name", city), ("count", "10"), ("language", "en"), ("format", "json")])
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let results = geo.results.unwrap_or_default();
        let pick = match &hint {
            Some(h) => results.iter().find(|r| {
                r.country.as_deref().is_some_and(|c| c.to_lowercase().contains(h)) || r.admin1.as_deref().is_some_and(|a| a.to_lowercase().contains(h))
            }),
            None => None,
        }
        .or(results.first())
        .ok_or_else(|| anyhow!("no place called \"{name}\""))?;
        let label = match &pick.country {
            Some(c) => format!("{}, {c}", pick.name),
            None => pick.name.clone(),
        };
        let place = Place { label, lat: pick.latitude, lon: pick.longitude };
        self.places.lock().unwrap().insert(key, place.clone());
        Ok(place)
    }

    async fn open_meteo(&self, place: &Place) -> Result<Report> {
        let url = "https://api.open-meteo.com/v1/forecast";
        let v: Value = self
            .http
            .get(url)
            .query(&[
                ("latitude", place.lat.to_string()),
                ("longitude", place.lon.to_string()),
                ("current", "temperature_2m,apparent_temperature,precipitation,weather_code,wind_speed_10m,relative_humidity_2m".into()),
                ("hourly", "temperature_2m,precipitation_probability,precipitation,weather_code".into()),
                ("daily", "weather_code,temperature_2m_max,temperature_2m_min,precipitation_sum,precipitation_probability_max,sunrise,sunset".into()),
                ("timezone", "auto".into()),
                ("forecast_days", "4".into()),
            ])
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Report::from_open_meteo(&place.label, &v)
    }

    async fn wttr(&self, place: &str) -> Result<String> {
        let v: Value = self.http.get(format!("https://wttr.in/{}?format=j1", urlencode(place))).send().await?.error_for_status()?.json().await?;
        let cur = &v["current_condition"][0];
        let desc = cur["weatherDesc"][0]["value"].as_str().unwrap_or("?");
        let mut s = format!(
            "{place} (wttr.in, less precise): now {desc}, {}°C (feels {}°C), precipitation {} mm.",
            cur["temp_C"].as_str().unwrap_or("?"),
            cur["FeelsLikeC"].as_str().unwrap_or("?"),
            cur["precipMM"].as_str().unwrap_or("?")
        );
        if let Some(hours) = v["weather"][0]["hourly"].as_array() {
            s.push_str(" Today every 3 h:");
            for h in hours {
                let t: u32 = h["time"].as_str().unwrap_or("0").parse().unwrap_or(0);
                s.push_str(&format!(" {:02}:00 {}mm {}%;", t / 100, h["precipMM"].as_str().unwrap_or("?"), h["chanceofrain"].as_str().unwrap_or("?")));
            }
        }
        Ok(s)
    }
}

impl Default for Weather {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for Weather {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "weather".into(),
            description: "Real weather from a forecast service: conditions now, the next 48 hours hour by hour, when rain starts or \
                          stops, and the next days. Use it for any question about weather, rain, snow, temperature, wind, sunrise \
                          or sunset. Leave `place` out for the user's home."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "place": { "type": "string", "description": "A city, optionally with its country (\"Lyon, France\"). Omit for home." }
                }
            }),
        }
    }

    fn risk(&self, _: &Value) -> Risk {
        Risk::Read
    }

    async fn call(&self, args: Value, ctx: &Ctx) -> Outcome {
        let asked: Option<String> = opt_arg(&args, "place");
        let asked = asked.filter(|p| !placeholder(p));
        let Some(name) = asked.or_else(|| home(ctx)) else {
            return Outcome::failed("the user's home city isn't known yet: ask them which city is home (once they say, it will be remembered)");
        };
        let place = match self.place(&name).await {
            Ok(p) => p,
            // A name the geocoder doesn't know is not a weather outage: say which, so she asks instead of blaming it.
            Err(e) if format!("{e}").starts_with("no place called") => return Outcome::failed(format!("{e}: ask the user for the city's name")),
            Err(e) => {
                tracing::warn!("geocoding: {e:#}");
                return match self.wttr(&name).await {
                    Ok(text) => Outcome::ok(text),
                    Err(e2) => Outcome::failed(format!("both weather services failed (Open-Meteo: {e:#}; wttr.in: {e2:#}); a web search may still find it")),
                };
            }
        };
        let primary = self.open_meteo(&place).await.map(|r| r.render());
        match primary {
            Ok(text) => Outcome::ok(text),
            Err(e1) => {
                tracing::warn!("open-meteo: {e1:#}");
                match self.wttr(&name).await {
                    Ok(text) => Outcome::ok(text),
                    Err(e2) => Outcome::failed(format!("both weather services failed (Open-Meteo: {e1:#}; wttr.in: {e2:#}); a web search may still find it")),
                }
            }
        }
    }
}

/// Words that stand for a place without naming one: "home", "user's home", "my current location", "where I live".
fn placeholder(p: &str) -> bool {
    const GENERIC: &[&str] = &[
        "home", "here", "house", "place", "city", "town", "unknown", "current", "location", "local", "area", "near", "nearby", "user", "users", "s", "the", "my", "your", "their", "at", "where", "i", "you", "live", "am", "is", "me",
    ];
    crate::matching::norm(p).split(' ').all(|w| w.is_empty() || GENERIC.contains(&w))
}

/// Home: what the app passed in, else what was saved since (set_home writes the settings file).
fn home(ctx: &Ctx) -> Option<String> {
    ctx.home.clone().or_else(|| capri_core::Settings::load().ok()?.home)
}

/// Saves where home is, so "the weather at home" works from now on.
pub struct SetHome;

#[async_trait]
impl Tool for SetHome {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "set_home".into(),
            description: "Remember the user's home city (for the weather and anything \"at home\"). Use it when they tell you \
                          where they live or where home is."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": { "place": { "type": "string", "description": "City, optionally with its country" } },
                "required": ["place"]
            }),
        }
    }

    async fn call(&self, args: Value, _ctx: &Ctx) -> Outcome {
        let place: String = match crate::tool::arg(&args, "place") {
            Ok(p) => p,
            Err(o) => return o,
        };
        if placeholder(&place) {
            return Outcome::failed(format!("\"{place}\" isn't a place name: ask the user which city is home"));
        }
        // Check it's a real place, and the one they named (the geocoder will happily turn "home" into "Homyel").
        let found = match Weather::new().place(&place).await {
            Ok(p) => p.label,
            Err(e) => return Outcome::failed(format!("couldn't find \"{place}\": {e:#}")),
        };
        if !same_place(place.split(',').next().unwrap_or(&place), &found) {
            return Outcome::failed(format!("the closest place to \"{place}\" is {found}, which may not be it: ask the user to confirm the city"));
        }
        let saved = (|| -> anyhow::Result<()> {
            let mut s = capri_core::Settings::load()?;
            s.home = Some(found.clone());
            s.save()
        })();
        match saved {
            Ok(()) => Outcome::ok(format!("home is now {found} (saved)")),
            Err(e) => Outcome::failed(format!("found {found} but couldn't save it: {e:#}")),
        }
    }
}

/// Whether the place the geocoder found is the one named: it starts with it, or it's the same name spelled another
/// way ("Odessa" is the geocoder's "Odesa"): two letters off at most, for names of 6 letters or more.
fn same_place(said: &str, found: &str) -> bool {
    let said = crate::matching::norm(said);
    let name = crate::matching::norm(found.split(',').next().unwrap_or(found));
    crate::matching::norm(found).starts_with(&said) || (said.chars().count() >= 6 && distance(&said, &name) <= 2)
}

/// Letters to change, add or drop to turn `a` into `b`.
fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut diag = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let next = (diag + usize::from(ca != *cb)).min(row[j] + 1).min(row[j + 1] + 1);
            diag = row[j + 1];
            row[j + 1] = next;
        }
    }
    row[b.len()]
}

/// The forecast, boiled down to what answers questions.
#[derive(Debug)]
struct Report {
    place: String,
    now_time: String,
    now: String,
    hours: Vec<Hour>,
    days: Vec<String>,
}

#[derive(Debug, Clone)]
struct Hour {
    time: String, // "2026-11-02T15:00", local
    temp: f64,
    mm: f64,
    chance: f64,
    code: i64,
}

impl Hour {
    fn wet(&self) -> bool {
        self.mm >= 0.1 || is_wet_code(self.code)
    }
}

impl Report {
    fn from_open_meteo(place: &str, v: &Value) -> Result<Self> {
        let cur = &v["current"];
        let now_time = cur["time"].as_str().context("no current time")?.to_string();
        let code = cur["weather_code"].as_i64().unwrap_or(-1);
        let now = format!(
            "{}, {:.0}°C (feels {:.0}°C), wind {:.0} km/h, humidity {:.0}%, precipitation {:.1} mm",
            describe(code),
            cur["temperature_2m"].as_f64().unwrap_or(f64::NAN),
            cur["apparent_temperature"].as_f64().unwrap_or(f64::NAN),
            cur["wind_speed_10m"].as_f64().unwrap_or(f64::NAN),
            cur["relative_humidity_2m"].as_f64().unwrap_or(f64::NAN),
            cur["precipitation"].as_f64().unwrap_or(0.0),
        );
        let h = &v["hourly"];
        let times = h["time"].as_array().context("no hourly")?;
        let mut hours = Vec::new();
        for (i, t) in times.iter().enumerate() {
            let time = t.as_str().unwrap_or_default().to_string();
            // From the current hour on (times compare as text: same format, same zone).
            if time.as_str() < &now_time[..13.min(now_time.len())] {
                continue;
            }
            hours.push(Hour {
                time,
                temp: h["temperature_2m"][i].as_f64().unwrap_or(f64::NAN),
                mm: h["precipitation"][i].as_f64().unwrap_or(0.0),
                chance: h["precipitation_probability"][i].as_f64().unwrap_or(0.0),
                code: h["weather_code"][i].as_i64().unwrap_or(-1),
            });
            if hours.len() >= 48 {
                break;
            }
        }
        let d = &v["daily"];
        let mut days = Vec::new();
        for (i, t) in d["time"].as_array().into_iter().flatten().enumerate() {
            days.push(format!(
                "{}: {}, {:.0}..{:.0}°C, rain {:.1} mm (chance {:.0}%), sunrise {}, sunset {}",
                t.as_str().unwrap_or("?"),
                describe(d["weather_code"][i].as_i64().unwrap_or(-1)),
                d["temperature_2m_min"][i].as_f64().unwrap_or(f64::NAN),
                d["temperature_2m_max"][i].as_f64().unwrap_or(f64::NAN),
                d["precipitation_sum"][i].as_f64().unwrap_or(0.0),
                d["precipitation_probability_max"][i].as_f64().unwrap_or(0.0),
                hhmm(d["sunrise"][i].as_str().unwrap_or("")),
                hhmm(d["sunset"][i].as_str().unwrap_or("")),
            ));
        }
        let wet_now = is_wet_code(code) || cur["precipitation"].as_f64().unwrap_or(0.0) > 0.0;
        let mut report = Self { place: place.to_string(), now_time, now, hours, days };
        if wet_now && let Some(first) = report.hours.first_mut() {
            // The current hour's figure can lag the live reading; trust the live reading.
            first.mm = first.mm.max(0.1);
        }
        Ok(report)
    }

    /// When the rain stops (raining now) or starts (dry now): the first hour that begins two in a row of the other kind,
    /// so a single dry hour in a rainy afternoon doesn't count as "it stops".
    fn rain_change(&self) -> String {
        let Some(first) = self.hours.first() else { return String::new() };
        let raining = first.wet();
        let flip = |from: usize, wet: bool| self.hours[from..].windows(2).position(|w| w[0].wet() != wet && w[1].wet() != wet).map(|i| i + from);
        let at = |i: usize| format!("{} {}", hhmm(&self.hours[i].time), day_word(&self.hours[i].time, &self.now_time));
        match (raining, flip(0, raining)) {
            (true, Some(i)) => format!("Raining now; it stops around {}.", at(i)),
            (true, None) => format!("Raining now, and it keeps raining for at least the next {} hours.", self.hours.len()),
            (false, Some(i)) => {
                let total: f64 = self.hours[i..].iter().take_while(|h| h.wet()).map(|h| h.mm).sum();
                let ends = match flip(i, true) {
                    Some(j) => format!("and stops around {}", at(j)),
                    None => "and is still going at the end of the forecast".into(),
                };
                format!("Not raining now. The next rain starts around {} {ends} ({total:.1} mm in all, chance up to {:.0}%).", at(i), self.hours[i..].iter().take_while(|h| h.wet()).map(|h| h.chance).fold(0.0, f64::max))
            }
            (false, None) => format!("Not raining now, and no rain expected in the next {} hours.", self.hours.len()),
        }
    }

    fn render(&self) -> String {
        // The rain answer first: it's what people ask about, and what must survive any shortening.
        let mut s = format!("{}: {} Now ({} local time): {}.\nNext 24 hours:", self.place, self.rain_change(), hhmm(&self.now_time), self.now);
        for h in self.hours.iter().take(24) {
            s.push_str(&format!(" {} {:.0}°C {:.1}mm {:.0}% {};", hhmm(&h.time), h.temp, h.mm, h.chance, describe(h.code)));
        }
        s.push_str("\nDays: ");
        s.push_str(&self.days.join(" | "));
        s
    }
}

fn hhmm(t: &str) -> String {
    t.get(11..16).unwrap_or(t).to_string()
}

/// "today", "tomorrow", or the date, comparing the local dates in the two timestamps.
fn day_word(t: &str, now: &str) -> String {
    let date = |s: &str| chrono::NaiveDate::parse_from_str(s.get(..10).unwrap_or(""), "%Y-%m-%d").ok();
    match (date(t), date(now)) {
        (Some(a), Some(b)) if a == b => "today".into(),
        (Some(a), Some(b)) if a == b.succ_opt().unwrap_or(b) => "tomorrow".into(),
        (Some(a), _) => format!("on {}", a.format("%A")),
        _ => String::new(),
    }
}

fn is_wet_code(code: i64) -> bool {
    matches!(code, 51..=67 | 80..=82 | 95..=99)
}

/// WMO weather codes, in words.
fn describe(code: i64) -> &'static str {
    match code {
        0 => "clear",
        1 => "mainly clear",
        2 => "partly cloudy",
        3 => "overcast",
        45 | 48 => "fog",
        51 | 53 | 55 => "drizzle",
        56 | 57 => "freezing drizzle",
        61 => "light rain",
        63 => "rain",
        65 => "heavy rain",
        66 | 67 => "freezing rain",
        71 => "light snow",
        73 => "snow",
        75 => "heavy snow",
        77 => "snow grains",
        80 => "light showers",
        81 => "showers",
        82 => "violent showers",
        85 | 86 => "snow showers",
        95 => "thunderstorm",
        96 | 99 => "thunderstorm with hail",
        _ => "unknown",
    }
}

fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => (b as char).to_string(),
            b' ' => "+".into(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(wet: &[bool]) -> Report {
        let hours = wet
            .iter()
            .enumerate()
            .map(|(i, &w)| Hour { time: format!("2026-11-02T{:02}:00", 12 + i), temp: 9.0, mm: if w { 0.6 } else { 0.0 }, chance: if w { 80.0 } else { 10.0 }, code: if w { 61 } else { 3 } })
            .collect();
        Report { place: "Lyon".into(), now_time: "2026-11-02T12:20".into(), now: String::new(), hours, days: vec![] }
    }

    #[test]
    fn home_spelled_another_way() {
        assert!(same_place("Odessa", "Odesa, Ukraine"));
        assert!(same_place("Porto", "Porto, Portugal"));
        assert!(same_place("Lyon", "Lyon, France"));
        assert!(!same_place("home", "Homyel, Belarus"));
        assert!(!same_place("Porto", "Poznan, Poland"));
        assert!(!same_place("Paris", "Parma, Italy"));
    }

    #[test]
    fn rain_stops_after_two_dry_hours() {
        // a lone dry hour at 14:00 doesn't count; it stops for real at 16:00
        let r = report(&[true, true, false, true, false, false, false]);
        assert!(r.rain_change().contains("stops around 16:00"), "{}", r.rain_change());
    }

    #[test]
    fn rain_starts_and_stops() {
        let r = report(&[false, false, false, true, true, true, false, false]);
        let s = r.rain_change();
        assert!(s.starts_with("Not raining now"), "{s}");
        assert!(s.contains("starts around 15:00 today") && s.contains("stops around 18:00 today"), "{s}");
    }

    #[test]
    fn placeholders_arent_places() {
        assert!(placeholder("home"));
        assert!(placeholder("My Home"));
        assert!(placeholder("here"));
        assert!(placeholder("user's home"));
        assert!(placeholder("my current location"));
        assert!(!placeholder("Porto"));
        assert!(!placeholder("Lyon, France"));
    }

    #[test]
    fn keeps_raining() {
        let r = report(&[true, true, true]);
        assert!(r.rain_change().contains("keeps raining"));
    }
}
