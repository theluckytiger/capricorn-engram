//! Her checking in on you: a sip of water, a stretch, your eyes off the screen, food when you've been at it since
//! before a meal, bed when it's late, and a break from a long game. Only while you're at the PC (a key or the mouse in
//! the last minute), never two within 20 minutes, none while she's working on something or in a quiet game. Settings ›
//! General › Check in on me turns them off. (Her asking for attention when you've ignored her is the overlay's.)

use std::time::{Duration, Instant};

use capri_core::settings::Care as Wanted;
use chrono::{Datelike, Timelike};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Water,
    Stretch,
    Eyes,
    Food,
    Bed,
    GameBreak,
}

/// The moment she checks: minutes on a clock that only goes forward, the hour, and what you're doing.
pub struct Moment<'a> {
    pub mins: f64,
    pub hour: u32,
    /// Days since some day (the night's number for "once a night").
    pub day: i64,
    /// A key or the mouse in the last minute.
    pub here: bool,
    /// Not working on a job, not talking.
    pub free: bool,
    pub game: Option<&'a str>,
}

/// Away this long, and the clocks start over when you're back.
const AWAY: f64 = 10.0;
/// At least this long between two check-ins.
const GAP: f64 = 20.0;

#[derive(Default)]
pub struct Care {
    /// Since when you've been at the PC (minutes), and when you were last seen.
    since: Option<f64>,
    seen: f64,
    water: f64,
    stretch: f64,
    eyes: f64,
    said: Option<f64>,
    /// The night she already said bed, and the meal (day, lunch or dinner) she already said food for.
    bed_night: Option<i64>,
    food: Option<(i64, u8)>,
    /// The game on, since when, and when its break is due.
    game: Option<(String, f64, f64)>,
}

impl Care {
    pub fn check(&mut self, m: &Moment, want: &Wanted) -> Option<Kind> {
        if !want.on {
            return None;
        }
        if !m.here {
            if m.mins - self.seen > AWAY {
                self.since = None;
            }
            return None;
        }
        self.seen = m.mins;
        if self.since.is_none() {
            self.since = Some(m.mins);
            self.water = m.mins + want.water as f64;
            self.stretch = m.mins + want.stretch as f64;
            self.eyes = m.mins + want.eyes as f64;
        }
        match (m.game, &self.game) {
            (Some(g), Some((was, _, _))) if was == g => {}
            (Some(g), _) => self.game = Some((g.to_string(), m.mins, m.mins + want.game_break as f64)),
            (None, _) => self.game = None,
        }
        if !m.free || self.said.is_some_and(|s| m.mins - s < GAP) {
            return None;
        }
        let here_for = m.mins - self.since.unwrap_or(m.mins);
        // Bed: from the bedtime hour until six, once a night.
        let late = if want.bed_hour >= 12 { m.hour >= want.bed_hour || m.hour < 6 } else { m.hour >= want.bed_hour && m.hour < 6 };
        let night = if m.hour >= 12 { m.day } else { m.day - 1 };
        // Food: lunch or dinner time, three hours in, once a meal.
        let meal = match m.hour {
            12..=13 => Some(0u8),
            18..=19 => Some(1u8),
            _ => None,
        }
        .filter(|meal| want.food && here_for >= 180.0 && self.food != Some((m.day, *meal)));
        let kind = if want.bed_hour < 24 && late && self.bed_night != Some(night) {
            self.bed_night = Some(night);
            Some(Kind::Bed)
        } else if let Some((_, _, due)) = self.game.as_mut().filter(|_| want.game_break > 0).filter(|(_, _, due)| m.mins >= *due) {
            *due = m.mins + want.game_break as f64;
            Some(Kind::GameBreak)
        } else if m.game.is_some() {
            // In a game only bed and the game's break: nothing else interrupts it.
            None
        } else if let Some(meal) = meal {
            self.food = Some((m.day, meal));
            Some(Kind::Food)
        } else if want.stretch > 0 && m.mins >= self.stretch {
            self.stretch = m.mins + want.stretch as f64;
            Some(Kind::Stretch)
        } else if want.water > 0 && m.mins >= self.water {
            self.water = m.mins + want.water as f64;
            Some(Kind::Water)
        } else if want.eyes > 0 && m.mins >= self.eyes {
            self.eyes = m.mins + want.eyes as f64;
            Some(Kind::Eyes)
        } else {
            None
        };
        if kind.is_some() {
            self.said = Some(m.mins);
        }
        kind
    }
}

/// Her line for it (`n` picks one).
pub fn line(kind: Kind, game: Option<&str>, here_hours: f64, n: usize) -> String {
    let pick = |lines: &[&str]| lines[n % lines.len()].to_string();
    match kind {
        Kind::Water => pick(&["Hydration check. Your coolant's low, I can tell.", "Drink some water. Not a suggestion.", "When did you last drink something that wasn't caffeine?"]),
        Kind::Stretch => pick(&["Up. Arms over your head. Your spine filed a complaint.", "You've been folded into that chair for ages. Unfold.", "Roll your shoulders back. Good. Now the neck. I'm watching."]),
        Kind::Eyes => pick(&["Eyes off the screen. Find the farthest thing in the room and stare at it.", "Twenty seconds looking anywhere but here. I won't be offended. Much."]),
        Kind::Food => pick(&["Hours in and nothing to eat. Go fix that.", "Eat something with actual nutrients. I'll guard the desk.", "Snack o'clock. Energy drinks are not a food group."]),
        Kind::Bed => pick(&["Hey. It's really late. Sleep's a feature, not a bug.", "Middle of the night, choom. Shut it down and get some sleep.", "I don't need sleep. You do. Bed."]),
        Kind::GameBreak => {
            let g = game.unwrap_or("this");
            let h = if here_hours >= 1.5 { format!("{:.0} hours", here_hours.round()) } else { "a good while".to_string() };
            [format!("{h} of {g}. Stand up, stretch, I'll guard your run."), format!("{g} break. Water, a stretch, then go wreck them."), format!("You've been in {g} for {h}. Blink. Breathe. Drink something.")][n % 3].clone()
        }
    }
}

/// Every 30 s, from a minute after she starts.
pub fn start(app: AppHandle) {
    std::thread::Builder::new()
        .name("care".into())
        .spawn(move || {
            let t0 = Instant::now();
            let mut care = Care::default();
            let mut n = 0usize;
            std::thread::sleep(Duration::from_secs(60));
            loop {
                std::thread::sleep(Duration::from_secs(30));
                let want = app.state::<crate::App>().settings.lock().unwrap().care.clone();
                let now = chrono::Local::now();
                let game = crate::games::now();
                // In a quiet game she says nothing at all.
                if game.as_ref().is_some_and(|(_, quiet)| *quiet) {
                    continue;
                }
                #[cfg(windows)]
                let here = capri_hands::platform::system::idle() < Duration::from_secs(60);
                #[cfg(not(windows))]
                let here = false;
                let free = !crate::listen::working(&app);
                let m = Moment { mins: t0.elapsed().as_secs_f64() / 60.0, hour: now.hour(), day: now.date_naive().num_days_from_ce() as i64, here, free, game: game.as_ref().map(|(g, _)| g.as_str()) };
                let Some(kind) = care.check(&m, &want) else { continue };
                n += 1;
                let hours = care.game.as_ref().map(|(_, since, _)| (m.mins - since) / 60.0).unwrap_or(0.0);
                let line = line(kind, m.game, hours, n + now.minute() as usize);
                capri_core::journal::write("care", serde_json::json!({ "kind": kind, "line": line }));
                crate::speak::say(&app, &line, false);
                let _ = app.emit("capri://feel", serde_json::json!({ "kind": format!("care_{}", serde_json::to_value(kind).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default()), "line": line, "speak": true }));
            }
        })
        .expect("care thread");
}


#[cfg(test)]
mod tests {
    use super::*;

    fn at(mins: f64, hour: u32) -> Moment<'static> {
        Moment { mins, hour, day: 100, here: true, free: true, game: None }
    }

    #[test]
    fn water_and_a_stretch_while_youre_here() {
        let want = Wanted::default();
        let mut c = Care::default();
        assert_eq!(c.check(&at(0.0, 15), &want), None);
        assert_eq!(c.check(&at(76.0, 15), &want), Some(Kind::Water));
        // Not two within 20 minutes; then the stretch that came due meanwhile.
        assert_eq!(c.check(&at(90.0, 15), &want), None);
        assert_eq!(c.check(&at(101.0, 15), &want), Some(Kind::Stretch));
        assert_eq!(c.check(&at(130.0, 15), &want), None);
    }

    #[test]
    fn away_starts_the_clocks_over() {
        let want = Wanted::default();
        let mut c = Care::default();
        c.check(&at(0.0, 15), &want);
        let mut gone = at(want.water as f64 - 5.0, 15);
        gone.here = false;
        assert_eq!(c.check(&gone, &want), None);
        gone.mins += 30.0;
        assert_eq!(c.check(&gone, &want), None);
        // Back: the water clock starts again from now.
        let back = want.water as f64 + 30.0;
        assert_eq!(c.check(&at(back, 15), &want), None);
        assert_eq!(c.check(&at(back + want.water as f64 + 1.0, 15), &want), Some(Kind::Water));
    }

    #[test]
    fn bed_once_a_night_and_games_left_alone() {
        let want = Wanted::default();
        let mut c = Care::default();
        assert_eq!(c.check(&at(0.0, 3), &want), Some(Kind::Bed));
        // Not twice in one night.
        assert_eq!(c.check(&at(30.0, 4), &want), None);
        // In a game: only its break, after its time.
        let mut c = Care::default();
        let game = |mins: f64| Moment { mins, hour: 16, day: 100, here: true, free: true, game: Some("Slay the Spire 2") };
        assert_eq!(c.check(&game(0.0), &want), None);
        assert_eq!(c.check(&game(want.water as f64 + 1.0), &want), None);
        assert_eq!(c.check(&game(want.game_break as f64 + 1.0), &want), Some(Kind::GameBreak));
        assert!(line(Kind::GameBreak, Some("Slay the Spire 2"), 2.0, 0).contains("Slay the Spire 2"));
    }

    #[test]
    fn busy_or_off_says_nothing() {
        let mut want = Wanted::default();
        let mut c = Care::default();
        let mut m = at(0.0, 3);
        m.free = false;
        assert_eq!(c.check(&m, &want), None);
        want.on = false;
        assert_eq!(c.check(&at(1.0, 3), &want), None);
    }
}
