//! Where a request goes: her fast path when the whole sentence is a plain command, else her agent on the `fast`
//! model, then the `smart` model when the first one got stuck (only if cloud is allowed and the day's budget isn't
//! spent). Her words come last, written from what actually happened.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use capri_hands::{Ctx, Hands, Status};
use serde::Serialize;
use serde_json::json;

use crate::agent::{self, Events, Gate, Limits, Run, Step};
use crate::fastpath::{self, Speak};
use crate::llm::{Effort, Model, Msg, Req};
use crate::{lines, persona};

/// Turns a smart brain may take on one job: enough that it ends by finishing, by your stop, or by her 25 minutes, not
/// by counting.
const SMART_TURNS: usize = 200;

pub struct Brain {
    pub hands: Hands,
    pub ctx: Ctx,
    pub fast: Arc<dyn Model>,
    pub smart: Option<Arc<dyn Model>>,
    /// Her eyes' model when it isn't `fast` (to free the GPU of it too).
    eyes: Option<Arc<dyn Model>>,
    pub cloud: bool,
    pub spicy: bool,
    pub daily_cloud: u32,
    cloud_used: Mutex<(chrono::NaiveDate, u32)>,
    recent: Mutex<VecDeque<Turn>>,
    /// "Did you mean Stremio?", "Force quit it?": what she'd do on a yes (the tool, its arguments, what she says once
    /// it's done), and when she asked.
    offer: Mutex<Option<Offer>>,
    /// Her mood (from how she's treated: thrown around, patted…), what it is, why, until when, and whether she's
    /// been told why already (after that, only the mood: she brought up the patting in every reply).
    mood: Mutex<Option<(String, String, Instant, bool)>>,
    /// A game is running: her local model stays off the GPU (fast path and cloud only). Shared with her eyes.
    gaming: Arc<AtomicBool>,
}

/// One exchange, kept a little while so follow-ups ("open his channel", "what did you just do?") make sense.
#[derive(Debug, Clone)]
struct Turn {
    at: Instant,
    asked: String,
    said: String,
    did: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Answer {
    /// What she says.
    pub line: String,
    pub steps: Vec<Step>,
    /// fast | agent | chat
    pub via: &'static str,
    pub model: Option<String>,
    pub ms: u128,
}

impl Brain {
    pub fn new(hands: Hands, ctx: Ctx, fast: Arc<dyn Model>, smart: Option<Arc<dyn Model>>, cloud: bool) -> Self {
        Self {
            hands,
            ctx,
            fast,
            smart,
            eyes: None,
            cloud,
            spicy: false,
            daily_cloud: 200,
            cloud_used: Mutex::new((chrono::Local::now().date_naive(), 0)),
            recent: Mutex::new(VecDeque::new()),
            offer: Mutex::new(None),
            mood: Mutex::new(None),
            gaming: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Eyes in the cloud she may use only on your yes, one look at a time (`name`: what she calls them when asking).
    pub fn with_cloud_eyes(mut self, model: Arc<dyn Model>, name: &str) -> Self {
        let eyes = crate::eyes::ModelEyes { model, gaming: self.gaming.clone() };
        self.ctx.cloud_eyes = Some((capri_hands::eyes::Eyes(Arc::new(eyes)), name.to_string()));
        self
    }

    /// Gives her eyes: `model` looks at windows for `see` and for clicking by looking. Only a model she may show the
    /// screen to (a local one, unless the user allows screenshots in the cloud).
    pub fn with_eyes(mut self, model: Arc<dyn Model>) -> Self {
        if model.id() != self.fast.id() {
            self.eyes = Some(model.clone());
        }
        let eyes = crate::eyes::ModelEyes { model, gaming: self.gaming.clone() };
        self.ctx.eyes = Some(capri_hands::eyes::Eyes(Arc::new(eyes)));
        self
    }

    /// Whether a game is running.
    pub fn is_gaming(&self) -> bool {
        self.gaming.load(Ordering::Relaxed)
    }

    /// A game started or ended. Starting one frees the GPU of her local models at once.
    pub async fn set_gaming(&self, on: bool) {
        self.gaming.store(on, Ordering::Relaxed);
        if on {
            self.unload().await;
        }
    }

    /// Her online brains get ready in the background, before her first request (Cursor's agent takes 15 s to start).
    pub async fn warm(&self) {
        for m in [Some(&self.fast), self.smart.as_ref()].into_iter().flatten().filter(|m| !m.local()) {
            m.warm().await;
        }
    }

    /// Frees the GPU of every local model of hers (a game started, or she's quitting).
    pub async fn unload(&self) {
        for m in [Some(&self.fast), self.smart.as_ref(), self.eyes.as_ref()].into_iter().flatten().filter(|m| m.local()) {
            m.unload().await;
        }
    }

    pub async fn ask(&self, text: &str, gate: &dyn Gate, events: &dyn Events) -> Answer {
        let start = Instant::now();
        let mut note = String::new();

        let request = events.request();
        // The answer to her "did you mean…?": yes does it, no asks for the name again; anything else is a new request.
        let offer = self.offer.lock().unwrap().take().filter(|o| o.at.elapsed().as_secs() < 120);
        if let Some(Offer { tool, args, done, no, .. }) = offer {
            match yes_no(text) {
                Some(true) => {
                    // A look in the cloud she asked about: this one call may use the cloud's eyes.
                    let ctx = if matches!(tool.as_str(), "see" | "click") && self.ctx.cloud_eyes.is_some() { capri_hands::Ctx { use_cloud_eyes: true, ..self.ctx.clone() } } else { self.ctx.clone() };
                    let outcome = self.hands.call(&tool, args.clone(), &ctx).await;
                    events.note("did you mean: yes", json!({ "tool": tool, "args": args, "status": outcome.status, "summary": outcome.summary }));
                    let line = if outcome.status == Status::Failed {
                        tidy(outcome.summary.split('\n').next().unwrap_or(&outcome.summary))
                    } else if done.is_empty() {
                        // What it saw: said her way.
                        self.say(self.fast.as_ref(), text, &outcome.summary, request).await.unwrap_or_else(|| tidy(&outcome.summary))
                    } else {
                        done
                    };
                    let step = Step { tool, args, outcome, ms: start.elapsed().as_millis() };
                    self.remember(text, &line, std::slice::from_ref(&step));
                    return Answer { line, steps: vec![step], via: "fast", model: None, ms: start.elapsed().as_millis() };
                }
                Some(false) => {
                    let line = no;
                    self.remember(text, &line, &[]);
                    return Answer { line, steps: vec![], via: "fast", model: None, ms: start.elapsed().as_millis() };
                }
                None => {}
            }
        }
        // Blackwall mode, by voice (only said plainly: it's her asking-first that goes).
        if let Some(on) = blackwall(text) {
            let saved = capri_core::Settings::update(|s| s.blackwall = on);
            events.note("blackwall", json!({ "on": on, "saved": saved.is_ok() }));
            let line = match (on, saved.is_ok()) {
                (true, true) => "Blackwall's up. I stop asking. Money, wiping things, Windows security and code off the net, I'll still check.".to_string(),
                (false, true) => "Blackwall's down. I'll ask before anything risky again.".to_string(),
                _ => "I couldn't change it. Settings, maybe?".to_string(),
            };
            self.remember(text, &line, &[]);
            return Answer { line, steps: vec![], via: "yours", model: None, ms: start.elapsed().as_millis() };
        }
        // Make her yours, by voice.
        if let Some(name) = call_me(text) {
            let saved = capri_core::Settings::update(|s| s.name = Some(name.clone()));
            events.note("call me", json!({ "name": name, "saved": saved.is_ok() }));
            let line = if saved.is_ok() { format!("{name}. Got it. I'll call you that. Mostly.") } else { "I tried to write that down and couldn't. Settings, maybe?".into() };
            self.remember(text, &line, &[]);
            return Answer { line, steps: vec![], via: "yours", model: None, ms: start.elapsed().as_millis() };
        }
        if let Some(fact) = remember_that(text) {
            let saved = capri_core::Settings::update(|s| {
                if !s.yours.memories.iter().any(|m| m.eq_ignore_ascii_case(&fact)) {
                    s.yours.memories.push(fact.clone());
                }
            });
            events.note("remember", json!({ "fact": fact, "saved": saved.is_ok() }));
            let line = if saved.is_ok() { "Filed away. In all of my minds.".to_string() } else { "I couldn't write it down. Settings, maybe?".into() };
            self.remember(text, &line, &[]);
            return Answer { line, steps: vec![], via: "yours", model: None, ms: start.elapsed().as_millis() };
        }
        // "Sorry", while she's in a mood: she forgives you, her way.
        let sorry_for = if is_sorry(text) { self.mood.lock().unwrap().take() } else { None };
        if let Some((what, why, _, _)) = sorry_for {
            events.note("sorry", json!({ "mood": what }));
            let gaming = self.gaming.load(Ordering::Relaxed) && self.fast.local();
            let note = format!("They're saying sorry because you were {what} ({why}). Forgive them, grudgingly, in your way.");
            let line = if gaming { None } else { self.chat(self.fast.as_ref(), text, &note, request).await };
            let line = line.unwrap_or_else(|| "Fine. Forgiven. Don't push it.".into());
            self.remember(text, &line, &[]);
            return Answer { line, steps: vec![], via: "chat", model: None, ms: start.elapsed().as_millis() };
        }
        if forgets(text) {
            self.forget();
            events.note("forgot the conversation", json!({}));
            return Answer { line: "Done. Whatever we said, I never heard it.".into(), steps: vec![], via: "fast", model: None, ms: start.elapsed().as_millis() };
        }
        // "Go to my other screen": she moves; not chat.
        if wants_other_screen(text) {
            events.note("other screen", json!({}));
            let line = "On it.".to_string();
            self.remember(text, &line, &[]);
            return Answer { line, steps: vec![], via: "screen", model: None, ms: start.elapsed().as_millis() };
        }
        if let Some(f) = fastpath::parse(text) {
            let outcome = self.hands.call(f.tool, f.args.clone(), &self.ctx).await;
            events.note("fast path", json!({ "tool": f.tool, "args": f.args, "status": outcome.status, "summary": outcome.summary }));
            let step = Step { tool: f.tool.into(), args: f.args.clone(), outcome: outcome.clone(), ms: start.elapsed().as_millis() };
            // No app by that name but one close to it, or it won't close: she asks.
            if let Some((line, offer)) = offer_in(&outcome) {
                *self.offer.lock().unwrap() = Some(offer);
                let step = Step { tool: f.tool.into(), args: f.args.clone(), outcome: outcome.clone(), ms: start.elapsed().as_millis() };
                self.remember(text, &line, std::slice::from_ref(&step));
                return Answer { line, steps: vec![step], via: "fast", model: None, ms: start.elapsed().as_millis() };
            }
            if outcome.status != Status::Failed || f.tool != "open" {
                let line = match f.speak {
                    Speak::Line => lines::for_fast(&f, &outcome),
                    Speak::Summarize => {
                        // In a game, phrasing it mustn't load her local model onto the GPU: the cloud, or the result as is.
                        let writer = if self.gaming.load(Ordering::Relaxed) && self.fast.local() { self.smart_if_allowed().filter(|s| !s.local()) } else { Some(self.fast.clone()) };
                        let said = match writer {
                            Some(m) => self.say(m.as_ref(), text, &outcome.summary, request).await,
                            None => None,
                        };
                        said.unwrap_or_else(|| tidy(outcome.summary.split('\n').next().unwrap_or(&outcome.summary)))
                    }
                };
                self.remember(text, &line, std::slice::from_ref(&step));
                return Answer { line, steps: vec![step], via: "fast", model: None, ms: start.elapsed().as_millis() };
            }
            // The quick way wasn't sure; her brain gets the request, and what the quick way found.
            events.note("fast path unsure: her brain takes it", json!({}));
            note = format!("\n(Tried first: open \"{}\" → {}.)", f.args["name"].as_str().unwrap_or(""), outcome.summary);
        }

        // Just talking ("thanks", "how are you"): nothing for her hands, so it goes straight to her words. (The brain with her
        // tools only ever said so first, 1.5 to 3 s on each one.)
        if note.is_empty() && small_talk(text) && !self.gaming.load(Ordering::Relaxed) {
            events.step(&crate::StepEvent::Thinking { model: self.fast.id().to_string() });
            if let Some(line) = self.chat(self.fast.as_ref(), text, "", request).await {
                self.remember(text, &line, &[]);
                return Answer { line, steps: vec![], via: "chat", model: Some(self.fast.id().to_string()), ms: start.elapsed().as_millis() };
            }
        }

        // "Try again", "keep going": the last job again, whole, on the brain that does jobs. "Try again." on its own went
        // to the small local brain, which clicked Steam's Manage Downloads, and the job's own words were lost.
        let again = self.again(text);
        if let Some(job) = &again {
            events.note("the last job again", json!({ "job": job }));
        }
        let text: &str = again.as_deref().unwrap_or(text);
        // Her notes on how things worked before in the apps this is about.
        let notes = if self.ctx.dry_run { Vec::new() } else { capri_core::notes::relevant(text, None, 5) };
        let notes = if notes.is_empty() { String::new() } else { format!("\n(Your notes on what worked before, use them if they fit:\n{})", notes.iter().map(|n| format!("- {}: {}", n.app, n.how)).collect::<Vec<_>>().join("\n")) };
        let situation = format!("{}{}{note}{notes}", capri_hands::situation::describe(&self.ctx), self.recent_text());
        // While a game runs, a local model would take its GPU: only the cloud (when allowed) thinks for her then.
        let gaming = self.gaming.load(Ordering::Relaxed);
        let first = if gaming && self.fast.local() {
            match self.smart_if_allowed().filter(|s| !s.local()) {
                Some(s) => s,
                None => {
                    events.note("a game is on and there's no cloud model: no thinking", json!({}));
                    let line = "I'm staying off your GPU while you play. Ask me after, or let me use the cloud.".to_string();
                    self.remember(text, &line, &[]);
                    return Answer { line, steps: vec![], via: "fast", model: None, ms: start.elapsed().as_millis() };
                }
            }
        } else {
            self.fast.clone()
        };
        // With a smart brain online, every job goes to it from the start. The local brain keeps the quick commands,
        // small talk, her spoken lines and her eyes.
        let planner = if !gaming && asks_to_do(text) && self.smart.as_ref().is_some_and(|s| !s.local()) { self.smart_if_allowed() } else { None };
        if let Some(p) = &planner {
            events.note("an agent job: the stronger brain plans it", json!({ "model": p.id(), "hard": hard_job(text) }));
        }
        let first = planner.clone().unwrap_or(first);
        // A job inside an app is many small turns (look, click, look): 14 ran out one click from Steam's Install button.
        // A stuck brain still stops early (the same step failing twice, four failures).
        // With nothing stronger to hand a job to, the brain doing it keeps at it the same way (eight turns and "I
        // couldn't" was the end of it on her local brain alone).
        let nothing_stronger = !(self.cloud && self.smart.as_ref().is_some_and(|s| s.id() != first.id()));
        let limits = if planner.is_some() {
            Limits { steps: SMART_TURNS, effort: Effort::Medium, persist: true }
        } else if nothing_stronger && asks_to_do(text) {
            Limits { steps: SMART_TURNS, effort: Effort::Low, persist: true }
        } else {
            Limits { steps: 8, effort: Effort::Low, persist: false }
        };
        let mut run = agent::run(first.as_ref(), &self.hands, &self.ctx, text, &situation, &limits, gate, events).await;
        let mut model = first.clone();
        // The online brain not answering (overloaded, a time-out) before it did anything: the one on this PC does it
        // ("my brain isn't answering" after two minutes, with a working one right here).
        if planner.is_some() && run.trouble.is_some() && run.steps.is_empty() && !(gaming && self.fast.local()) {
            events.note("the stronger brain didn't answer: the local one takes it", json!({ "trouble": run.trouble }));
            model = self.fast.clone();
            run = agent::run(model.as_ref(), &self.hands, &self.ctx, text, &situation, &Limits { steps: 8, effort: Effort::Low, persist: false }, gate, events).await;
        }

        // Asked to do something, and nothing was done: once more, told to act (it answered "only one video's showing"
        // without looking).
        if run.steps.is_empty() && run.trouble.is_none() && asks_to_do(text) {
            events.note("asked to act and nothing was done: again, told to act", json!({ "reply": run.summary }));
            let situation = format!("{situation}\n(This is a request to do something on the PC. Act on it with your tools; if you need to know what's on a window, look at it with `see` first, never guess. Don't answer without acting.)");
            let again = agent::run(model.as_ref(), &self.hands, &self.ctx, text, &situation, &limits, gate, events).await;
            run = again;
            if run.steps.is_empty() && run.trouble.is_none() {
                run.escalate = true;
            }
        }

        if run.escalate
            && !gaming
            && planner.is_none()
            && let Some(smart) = self.smart_if_allowed()
        {
            tracing::info!("escalating to {}: {:?}", smart.id(), run.trouble);
            events.note("escalating", json!({ "to": smart.id(), "trouble": run.trouble }));
            let tried = summarize_steps(&run.steps);
            let situation = format!("{situation}\n(A smaller model tried first and got stuck. What it did: {tried})");
            let limits = Limits { steps: SMART_TURNS, effort: Effort::Medium, persist: true };
            let second = agent::run(smart.as_ref(), &self.hands, &self.ctx, text, &situation, &limits, gate, events).await;
            let mut steps = run.steps;
            steps.extend(second.steps.clone());
            run = Run { steps, ..second };
            model = smart;
        }

        // Her brain ended on something she can offer (an app close to the name said, a force quit), and nothing after
        // worked: she asks.
        let offered = run.steps.iter().rposition(|s| offer_in(&s.outcome).is_some()).filter(|&i| run.steps[i..].iter().all(|s| s.outcome.status != Status::Ok)).and_then(|i| offer_in(&run.steps[i].outcome));
        if let Some((line, offer)) = offered {
            *self.offer.lock().unwrap() = Some(offer);
            self.remember(text, &line, &run.steps);
            return Answer { line, steps: run.steps, via: "agent", model: Some(model.id().to_string()), ms: start.elapsed().as_millis() };
        }
        let (line, via) = if run.steps.is_empty() && run.trouble.is_none() && asks_to_do(text) {
            // Asked to act, and still nothing done: she says so, in her words, and never that it's done.
            let note = "Nothing was done on the PC for this: no tool ran. Do NOT say or hint that it's done. Say you didn't manage it this time and ask them to say it another way.";
            let line = self.chat(model.as_ref(), text, note, request).await.filter(|l| !agent::claims_action(l)).unwrap_or_else(|| "I didn't get that one done. Say it another way?".into());
            (line, "chat")
        } else if run.steps.is_empty() && run.trouble.is_none() {
            // Nothing to do on the PC: talk to her, from what her brain worked out ("which city is home?"), never
            // from nothing (a made-up forecast, when home wasn't known).
            (self.chat(model.as_ref(), text, &run.summary, request).await.unwrap_or_else(|| tidy(&run.summary)), "chat")
        } else {
            let facts = facts(&run);
            // If her model can't even be reached, the error isn't something to read out: say it plainly.
            let fallback = match (&run.trouble, run.steps.is_empty()) {
                (Some(t), true) => format!("My brain isn't answering right now ({}). Try me again in a moment.", t.split(':').next().unwrap_or(t)),
                _ => facts.clone(),
            };
            // Her words come from her own (local) brain, whoever did the job: her voice stays hers, and a cloud model
            // that spends its few words on hidden reasoning left her reading out "Tool results: …". In a game, a
            // local one would take the GPU: then the one that did it.
            let writer = if self.ctx.dry_run || !(self.gaming.load(Ordering::Relaxed) && self.fast.local()) { self.fast.clone() } else { model.clone() };
            let said = match self.say(writer.as_ref(), text, &facts, request).await {
                Some(l) => Some(l),
                None if writer.id() != model.id() => self.say(model.as_ref(), text, &facts, request).await,
                None => None,
            };
            (said.unwrap_or(fallback), "agent")
        };
        self.remember(text, &line, &run.steps);
        if via == "agent" && !self.ctx.dry_run {
            learn(text, &run.steps);
        }
        Answer { line, steps: run.steps, via, model: Some(model.id().to_string()), ms: start.elapsed().as_millis() }
    }

    fn smart_if_allowed(&self) -> Option<Arc<dyn Model>> {
        let smart = self.smart.clone()?;
        if !smart.local() {
            if !self.cloud {
                return None;
            }
            let mut used = self.cloud_used.lock().unwrap();
            let today = chrono::Local::now().date_naive();
            if used.0 != today {
                *used = (today, 0);
            }
            if used.1 >= self.daily_cloud {
                tracing::warn!("cloud budget for today is spent");
                return None;
            }
            used.1 += 1;
        }
        Some(smart)
    }

    /// Her reaction to something on screen while watching along with you: one short line, in her voice, from her own
    /// (local) brain; nothing while a game has her GPU and she has no cloud brain.
    pub async fn aside(&self, seen: &str) -> Option<String> {
        if self.gaming.load(Ordering::Relaxed) && self.fast.local() {
            return None;
        }
        let note = format!("You're watching this together with them right now. On screen: {seen}\nReact in ONE short line, in your voice, like a friend watching along: a quip, a guess, a reaction. Don't describe it back.");
        self.chat(self.fast.as_ref(), "(watching along)", &note, 0).await
    }

    /// Her mood (`minutes` long): what, and why.
    pub fn set_mood(&self, what: &str, why: &str, minutes: u64) {
        *self.mood.lock().unwrap() = Some((what.into(), why.into(), Instant::now() + std::time::Duration::from_secs(minutes * 60), false));
    }

    /// What she knows of you and her mood, for her prompts.
    fn hers(&self) -> String {
        let you = capri_core::Settings::load().map(|s| persona::about_you(&s)).unwrap_or_default();
        let mut mood = self.mood.lock().unwrap();
        if mood.as_ref().is_some_and(|m| Instant::now() > m.2) {
            *mood = None;
        }
        let m = mood
            .as_mut()
            .map(|(what, why, _, told)| {
                let m = if *told { persona::mood_quiet(what) } else { persona::mood(what, why) };
                *told = true;
                m
            })
            .unwrap_or_default();
        let bond = persona::bond(capri_core::bond::load().stage);
        format!("{you}{bond}{m}")
    }

    /// Her line, from the facts.
    async fn say(&self, model: &dyn Model, asked: &str, facts: &str, request: u64) -> Option<String> {
        let prompt = format!("{}{}", persona::speak_prompt(self.spicy), self.hers());
        let msg = Msg::user(format!("The user asked: {asked}\nWhat actually happened (facts): {facts}\nNow say it to them."));
        let req = Req { system: &prompt, msgs: &[msg], tools: &[], max_tokens: 160, effort: Effort::Low };
        let r = crate::llm::ask(model, req, "speak", request).await.ok()?;
        let line = tidy(&r.text);
        // A smart-brain essay pasted into speak: ask once more for one short spoken line.
        if looks_like_essay(&line) {
            let msg = Msg::user(format!(
                "The user asked: {asked}\nWhat actually happened (facts): {facts}\n\
                 Say that in ONE short spoken line in Capri's voice. Only facts above — no song, price or window name that isn't there."
            ));
            let req = Req { system: &prompt, msgs: &[msg], tools: &[], max_tokens: 120, effort: Effort::Low };
            if let Ok(again) = crate::llm::ask(model, req, "speak again", request).await {
                let short = tidy(&again.text);
                if !short.is_empty() && !looks_like_essay(&short) {
                    return Some(short);
                }
            }
        }
        Some(line).filter(|t| !t.is_empty())
    }

    async fn chat(&self, model: &dyn Model, text: &str, notes: &str, request: u64) -> Option<String> {
        // Her satellite, one chat in ten.
        let lore = chrono::Local::now().timestamp_subsec_nanos().is_multiple_of(10);
        let prompt = format!("{}{}", persona::chat_prompt(self.spicy, lore), self.hers());
        let notes = if notes.trim().is_empty() { String::new() } else { format!("\nYour notes (true; answer from them): {}", notes.trim()) };
        // She knows what day and time it is, even just chatting.
        let now = chrono::Local::now().format("Now: %A %-d %B %Y, %H:%M (local time).");
        let msgs = [Msg::user(format!("{now}{}\nThe user says: {text}{notes}", self.recent_text()))];
        let req = || Req { system: &prompt, msgs: &msgs, tools: &[], max_tokens: 200, effort: Effort::Low };
        let r = match crate::llm::ask(model, req(), "chat", request).await {
            Ok(r) => r,
            // A busy online brain (a 503, "overloaded") usually answers a moment later.
            Err(_) => {
                tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
                crate::llm::ask(model, req(), "chat", request).await.ok()?
            }
        };
        let line = tidy(&r.text);
        // A small model parrots its last line back ("you threw me like a neon frisbee", four times running): asked
        // again without the lines it could copy.
        let said: Vec<String> = self.recent.lock().unwrap().iter().map(|t| t.said.clone()).collect();
        if said.iter().any(|s| repeats(&line, s)) {
            let msg = Msg::user(format!("{now}\nThe user says: {text}{notes}\nSay something new that answers exactly that."));
            let req = Req { system: &prompt, msgs: &[msg], tools: &[], max_tokens: 200, effort: Effort::Low };
            if let Ok(r) = crate::llm::ask(model, req, "chat again", request).await {
                let again = tidy(&r.text);
                if !again.is_empty() && !said.iter().any(|s| repeats(&again, s)) {
                    return Some(again);
                }
            }
        }
        Some(line).filter(|t| !t.is_empty())
    }

    /// Forgets the conversation so far (what she keeps of it: the last few turns).
    pub fn forget(&self) {
        self.recent.lock().unwrap().clear();
    }

    fn remember(&self, asked: &str, said: &str, steps: &[Step]) {
        let mut r = self.recent.lock().unwrap();
        r.push_back(Turn { at: Instant::now(), asked: asked.into(), said: said.into(), did: summarize_steps(steps) });
        while r.len() > 6 {
            r.pop_front();
        }
    }

    /// "Try again" (or "keep going", "do it again") after a job in the last ten minutes: that job's request, with what
    /// was said now.
    fn again(&self, text: &str) -> Option<String> {
        if !retry(text) {
            return None;
        }
        let r = self.recent.lock().unwrap();
        let last = r.iter().rev().find(|t| t.at.elapsed().as_secs() < 600 && (t.did != "nothing" || asks_to_do(&t.asked)))?;
        // A job that was itself a "try again" carries its own request already.
        Some(if last.asked.ends_with(')') && last.asked.contains(" (") { last.asked.clone() } else { format!("{} ({})", last.asked.trim(), text.trim()) })
    }

    fn recent_text(&self) -> String {
        let mut r = self.recent.lock().unwrap();
        r.retain(|t| t.at.elapsed().as_secs() < 600);
        if r.is_empty() {
            return String::new();
        }
        let lines: Vec<String> = r.iter().map(|t| format!("- user: \"{}\" → she did: {} → she said: \"{}\"", t.asked, t.did, t.said)).collect();
        format!("\nJust before (oldest first):\n{}", lines.join("\n"))
    }
}

/// "Try again", "again", "keep going", "do it again", "one more time": the last job once more.
pub fn retry(text: &str) -> bool {
    let t: String = text.to_lowercase().chars().map(|c| if c.is_alphanumeric() || c == ' ' || c == '\'' { c } else { ' ' }).collect();
    let words: Vec<&str> = t.split_whitespace().filter(|w| !matches!(*w, "capri" | "capricorn" | "please" | "pls" | "can" | "could" | "you" | "just" | "now" | "ok" | "okay" | "come" | "then")).collect();
    let said = words.join(" ");
    words.len() <= 6
        && ["try again", "try it again", "again", "retry", "do it again", "keep going", "keep trying", "go on", "carry on", "continue", "one more time", "try once more", "finish it", "finish the job", "don't give up", "dont give up"]
            .iter()
            .any(|p| said == *p || (p.contains(' ') && (said.starts_with(&format!("{p} ")) || said.ends_with(&format!(" {p}")))))
}

/// Whether `line` mostly says `before` again: most of the shorter one's words are in the other.
fn repeats(line: &str, before: &str) -> bool {
    let words = |s: &str| -> std::collections::HashSet<String> { s.split_whitespace().map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase()).filter(|w| w.chars().count() >= 3).collect() };
    let (a, b) = (words(line), words(before));
    let fewer = a.len().min(b.len());
    fewer >= 4 && a.intersection(&b).count() * 10 >= fewer * 6
}

/// A long formal reply that isn't her spoken voice (a smart-brain draft copied into speak).
fn looks_like_essay(line: &str) -> bool {
    let t = line.trim();
    t.len() > 280 || t.contains('\n') || t.matches(['.', '!', '?']).count() > 3
}

/// "Go to my other screen", "switch to the other monitor"…
fn wants_other_screen(text: &str) -> bool {
    let s: String = text.to_lowercase().chars().map(|c| if c.is_alphanumeric() { c } else { ' ' }).collect();
    let s = s.split_whitespace().filter(|w| !matches!(*w, "capri" | "capricorn" | "please" | "can" | "you" | "could" | "would")).collect::<Vec<_>>().join(" ");
    matches!(
        s.as_str(),
        "go to my other screen"
            | "go to the other screen"
            | "go to other screen"
            | "go to my other monitor"
            | "go to the other monitor"
            | "switch to my other screen"
            | "switch to the other screen"
            | "switch to my other monitor"
            | "switch to the other monitor"
            | "move to my other screen"
            | "move to the other screen"
            | "move to my other monitor"
            | "come to my other screen"
            | "jump to my other screen"
            | "fly to my other screen"
            | "go over to my other screen"
    )
}

/// What she'd do on a yes to her question: the tool and its arguments, what she says once it's done or on a no.
struct Offer {
    tool: String,
    args: serde_json::Value,
    done: String,
    no: String,
    at: Instant,
}

/// What a step's outcome offers to do on a yes ("did you mean…?", "force quit it?"), with her question.
fn offer_in(o: &capri_hands::Outcome) -> Option<(String, Offer)> {
    let at = Instant::now();
    if let Some(near) = o.data["did_you_mean"].as_str() {
        let no = "Okay. What's it called, then? Say it slowly, or how it starts.".to_string();
        return Some((format!("Did you mean {near}?"), Offer { tool: "open".into(), args: json!({ "name": near }), done: format!("Opening {near}."), no, at }));
    }
    let offer = &o.data["offer"];
    let pick = |k: &str, or: &str| offer[k].as_str().unwrap_or(or).to_string();
    Some((offer["ask"].as_str()?.to_string(), Offer { tool: offer["tool"].as_str()?.to_string(), args: offer["args"].clone(), done: pick("done", "Done."), no: pick("no", "Okay. Leaving it."), at }))
}

/// Whether something said is a yes or a no to her question (the words are in `capri_core::answer`).
pub use capri_core::answer::yes_no;

/// A request to do something on the PC ("click the fifth video", "can you open Stremio", "go back in Stremio"): her
/// brain has to act on it with a tool, never answer it from chat (a chat answer to one claimed "Stremio's open").
pub fn asks_to_do(text: &str) -> bool {
    let s: String = text.to_lowercase().chars().map(|c| if c.is_alphanumeric() || c == '\'' { c } else { ' ' }).collect();
    let words: Vec<&str> = s.split_whitespace().collect();
    const FILLER: &[&str] = &["need", "needs", "to", "have", "supposed", "should", "must", "gotta", "are", "you're", "capri", "capricorn", "copy", "hey", "ok", "okay", "so", "um", "uh", "please", "pls", "yo", "and", "now", "just", "no", "can", "could", "would", "will", "you", "u", "i", "want", "go", "for", "me", "quickly", "real", "quick", "i'd", "id", "like", "i'm", "im", "gonna", "wanna", "let's", "lets", "maybe"];
    const DO: &[&str] = &[
        "open", "pull", "fire", "boot", "load", "close", "click", "press", "tap", "hit", "play", "listen", "pause", "resume", "skip", "mute", "unmute", "type", "write", "search", "find", "look", "check", "show", "scroll", "navigate", "select", "choose", "pick", "start", "launch", "run", "install", "uninstall", "download", "delete", "remove", "rename", "move", "copy", "paste", "send", "minimize", "maximize", "restore", "switch", "turn", "set", "put", "bring", "hide", "drag", "focus", "refresh", "reload", "watch", "back", "enter", "exit", "quit", "join", "leave", "message", "cancel", "shut", "shutdown", "restart", "reboot", "lock", "remind", "snap", "pin", "unpin",
    ];
    // The first word that isn't filler is what they want done ("you look tired" isn't a request), or an action after
    // "then" or "and" ("go to loaded.com and then search for…").
    let first = words.iter().find(|w| !FILLER.contains(w)).is_some_and(|w| DO.contains(w));
    let later = words.windows(2).any(|p| matches!(p[0], "then" | "and") && DO.contains(&p[1]));
    first || later
}

/// Just talking: a greeting, thanks, a reaction, asking how she is. Only what's plainly that; anything else is for the brain
/// with her tools to look at.
pub fn small_talk(text: &str) -> bool {
    let s: String = text.to_lowercase().replace('\u{2019}', "'").chars().map(|c| if c.is_alphanumeric() || c == '\'' { c } else { ' ' }).collect();
    let words: Vec<&str> = s.split_whitespace().filter(|w| !matches!(*w, "capri" | "capricorn" | "copy")).collect();
    if words.is_empty() || words.len() > 7 {
        return false;
    }
    let said = words.join(" ");
    const WHOLE: &[&str] = &[
        "hi", "hello", "hey", "hiya", "yo", "sup", "nice", "cool", "awesome", "great", "perfect", "lol", "haha", "lmao", "good job", "nice job", "well done", "good girl", "good morning", "good afternoon", "good evening", "good night", "gn",
        "love you", "i love you", "bye", "goodbye", "see you", "see ya", "what's up", "whats up", "how are you", "how are you doing", "how's it going", "hows it going", "how's your day", "how's your day going", "how's your day been", "how was your day",
        "who are you", "what are you", "what's your name", "you're funny", "you're cute", "you're the best", "you're great", "you're amazing",
    ];
    if WHOLE.contains(&said.as_str()) {
        return true;
    }
    // "Thanks", "thank you so much", "thanks a lot, girl": and nothing else after it.
    let rest = said.strip_prefix("thank you").or_else(|| said.strip_prefix("thanks"));
    rest.is_some_and(|r| r.split_whitespace().all(|w| matches!(w, "so" | "much" | "a" | "lot" | "very" | "again" | "for" | "that" | "the" | "help" | "girl" | "babe" | "bro" | "choom")))
}

/// A job for her stronger brain from the start (when it's set up): working inside a window ("click the fifth video",
/// "the search bar in Stremio") or several steps in one ("open Stremio and search Jujutsu Kaisen").
pub fn hard_job(text: &str) -> bool {
    if !asks_to_do(text) {
        return false;
    }
    // Dots in hostnames stay ("loaded.com"); sentence-ending dots become spaces so "album." still matches " album ".
    let s = hard_job_text(text);
    const INSIDE: &[&str] = &[
        " install ", " uninstall ", " download ", " click ", " press ", " tap ", " select ", " scroll ", " thumbnail", " button", " tab ", " search bar", " search box", " menu", " first ", " second ", " third ", " fourth ", " fifth ", " sixth ", " last ", " 1st ", " 2nd ", " 3rd ", " 4th ", " 5th ", " in my browser", " in the browser", " page ", " link", " video ", " videos ", " go back", " check ", " download",
    ];
    const THEN: &[&str] = &[
        " and then ", " then ", " and search ", " and find ", " then search ", " then find ", " and play ", " and open ", " and click ",
        " and type ", " and press ", " and tell ", " and check ", " and download ",
    ];
    const SITE: &[&str] = &[".com", ".net", ".org", ".io", ".gg", ".tv"];
    if INSIDE.iter().any(|w| s.contains(w)) || THEN.iter().any(|w| s.contains(w)) || SITE.iter().any(|t| s.contains(t)) {
        return true;
    }
    // "the latest Skepta album": needs a current fact, then play — the 8B once played a random YouTube hit instead.
    if fresh_album(&s) {
        return true;
    }
    // "Search for X in Stremio", "find Y on Epic", "play Z on Stremio": in/on a place that isn't a music site the
    // quick brain handles with play_music.
    in_app_or_site(&s)
}

/// Lowercased, punctuation as spaces, except dots inside hostnames (letter.digit on both sides).
fn hard_job_text(text: &str) -> String {
    let chars: Vec<char> = text.to_lowercase().chars().collect();
    let mut out = String::with_capacity(chars.len() + 2);
    out.push(' ');
    for (i, &c) in chars.iter().enumerate() {
        if c == '.' {
            let keep = i > 0 && chars[i - 1].is_alphanumeric() && chars.get(i + 1).is_some_and(|a| a.is_alphanumeric());
            out.push(if keep { '.' } else { ' ' });
        } else if matches!(c, ',' | '?' | '!' | ';' | ':' | '"' | '\'' | '(' | ')') {
            out.push(' ');
        } else {
            out.push(c);
        }
    }
    out.push(' ');
    out
}

/// "latest/new album|ep|mixtape": look up which release it is, then play it.
fn fresh_album(s: &str) -> bool {
    let kind = [" album ", " ep ", " mixtape ", " lp "].iter().any(|w| s.contains(w));
    let fresh = [" latest ", " newest ", " new ", " recent "].iter().any(|w| s.contains(w));
    kind && fresh
}

/// "search/find/play … in/on <place>": a job inside an app or on a site, unless the place is YouTube/Spotify music.
fn in_app_or_site(s: &str) -> bool {
    const ACT: &[&str] = &[" search ", " find ", " play ", " look "];
    if !ACT.iter().any(|a| s.contains(a)) {
        return false;
    }
    // Music / video play targets the quick play_music path, not a multi-step app job.
    const MUSIC: &[&str] = &[
        " on youtube ", " on youtube music ", " on yt ", " on spotify ", " on apple music ", " on tidal ",
        " on youtube_music ", " youtube music",
    ];
    if s.contains(" play ") && MUSIC.iter().any(|m| s.contains(m)) {
        return false;
    }
    // Plain web search ("search for cats on google") stays on the quick brain.
    const WEB: &[&str] = &["google", "bing", "duckduckgo", "ddg", "brave"];
    // "in/on" then a real place word (not "in my browser", already in INSIDE).
    for prep in [" in ", " on "] {
        let mut rest = s;
        while let Some(i) = rest.find(prep) {
            let after = rest[i + prep.len()..].trim_start();
            let place = after.split_whitespace().next().unwrap_or("").trim_matches(|c: char| !c.is_alphanumeric());
            const SKIP: &[&str] = &["my", "the", "a", "an", "this", "that", "your", "our", "his", "her", "its", "me", "you", "it", "here", "there", "mind", "case", "fact", "general", "loop", "repeat", "shuffle", "full", "fullscreen", "background", "order", "time", "now", "top", "bottom", "screen", "window", "pc", "computer", "desktop", "browser"];
            if !place.is_empty() && !SKIP.contains(&place) && !WEB.contains(&place) {
                return true;
            }
            rest = &rest[i + prep.len()..];
        }
    }
    false
}

/// "Call me Max", "my name is Max", "from now on call me Max": the name, as said (a word or three).
fn call_me(text: &str) -> Option<String> {
    let lower = text.to_lowercase();
    let start = ["from now on call me ", "you can call me ", "call me ", "my name is ", "my name's "].iter().find_map(|p| lower.find(p).map(|i| i + p.len()))?;
    // Only at the start of what was said (after her name): "I said call me later" isn't a name.
    let before = lower[..start].split_whitespace().filter(|w| !matches!(w.trim_matches(|c: char| !c.is_alphanumeric()), "capri" | "capricorn" | "hey" | "ok" | "okay" | "please" | "so" | "and" | "now" | "from" | "on" | "call" | "me" | "you" | "can" | "my" | "name" | "is" | "name's")).count();
    if before > 0 || lower.len() != text.len() {
        return None;
    }
    let name: String = text[start..].trim().trim_end_matches(['.', '!', '?', ',']).trim().to_string();
    let words: Vec<&str> = name.split_whitespace().collect();
    const NOT: &[&str] = &["when", "if", "later", "back", "at", "in", "after", "tomorrow", "tonight", "a", "an", "the", "on", "up", "out", "maybe", "please", "now", "soon", "again"];
    if words.is_empty() || words.len() > 3 || name.len() > 30 || NOT.contains(&words[0].to_lowercase().as_str()) {
        return None;
    }
    // "max" said lowercase by her ears: a capital to start each word.
    Some(words.iter().map(|w| { let mut c = w.chars(); c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default() }).collect::<Vec<_>>().join(" "))
}

/// "Remember that I hate mornings", "don't forget that my sister is called Ana": the thing to remember. Not "remember
/// to…" (a reminder) or a question.
fn remember_that(text: &str) -> Option<String> {
    let t = text.trim().trim_end_matches(['.', '!']);
    let lower = t.to_lowercase();
    let lower = lower.trim_start_matches("capri, ").trim_start_matches("capri ").trim_start_matches("please ");
    let offset = t.len() - lower.len();
    let start = ["remember that ", "don't forget that ", "do not forget that ", "remember, "].iter().find_map(|p| lower.strip_prefix(p).map(|_| p.len()))?;
    if lower.len() != t.len() - offset || t.ends_with('?') {
        return None;
    }
    let fact = t[offset + start..].trim();
    (fact.split_whitespace().count() >= 2).then(|| fact.to_string())
}

/// "Blackwall mode on", "turn off blackwall"…: on or off.
fn blackwall(text: &str) -> Option<bool> {
    let s: String = text.to_lowercase().chars().map(|c| if c.is_alphanumeric() { c } else { ' ' }).collect();
    let s = s.split_whitespace().filter(|w| !matches!(*w, "capri" | "capricorn" | "please" | "mode" | "the")).collect::<Vec<_>>().join(" ");
    match s.as_str() {
        "blackwall on" | "turn on blackwall" | "turn blackwall on" | "enable blackwall" | "activate blackwall" | "go blackwall" => Some(true),
        "blackwall off" | "turn off blackwall" | "turn blackwall off" | "disable blackwall" | "deactivate blackwall" => Some(false),
        _ => None,
    }
}

/// "Sorry", "I'm sorry", "my bad", "I apologize"…
fn is_sorry(text: &str) -> bool {
    let s: String = text.to_lowercase().chars().filter(|c| c.is_alphanumeric() || *c == ' ' || *c == '\'').collect();
    let s = s.trim_start_matches("capri ").trim();
    // "Sorry, what time is it?" is a question with manners, not an apology.
    const ASKING: &[&str] = &["what", "when", "where", "who", "why", "how", "can", "could", "would", "will", "is", "are", "do", "does", "open", "play", "set", "turn", "close", "show", "tell"];
    ["sorry", "i'm sorry", "im sorry", "i am sorry", "my bad", "i apologize", "i apologise", "sorry capri", "okay sorry", "ok sorry", "sorry sorry", "i'm so sorry", "so sorry"].iter().any(|p| {
        s == *p || s.strip_prefix(&format!("{p} ")).is_some_and(|rest| !text.trim_end().ends_with('?') && !rest.split_whitespace().next().is_some_and(|w| ASKING.contains(&w)))
    })
}

/// "Forget our conversation", "forget what we talked about", "forget everything I said"…
fn forgets(text: &str) -> bool {
    let s: String = text.to_lowercase().chars().filter(|c| c.is_alphanumeric() || *c == ' ' || *c == '\'').collect();
    let s = s.trim_start_matches("please ").trim_start_matches("can you ").trim_start_matches("just ");
    let Some(rest) = s.strip_prefix("forget ") else { return false };
    let rest = rest.trim_end_matches(" please").trim();
    ["everything", "all that", "all of that", "everything i said", "everything we said", "what we said", "what we talked about", "what i said", "what i told you", "our conversation", "our conversations", "this conversation", "the conversation", "everything we talked about"]
        .contains(&rest)
}

/// Spoken lines: one paragraph, a capital to start, three sentences at most (small models ramble).
pub fn tidy(text: &str) -> String {
    let flat = persona::no_bro(&plain(text).split_whitespace().collect::<Vec<_>>().join(" "));
    let mut out = String::new();
    let mut sentences = 0;
    let chars: Vec<char> = flat.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        out.push(c);
        let end = matches!(c, '.' | '!' | '?') && chars.get(i + 1).is_none_or(|n| *n == ' ');
        if end {
            sentences += 1;
            if sentences == 3 {
                break;
            }
        }
    }
    let mut c = out.trim().chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

/// Markdown a model slips into a reply (`**bold**`, `- ` lists, `# headings`, backticks) taken out: her panel shows
/// plain text and her voice reads every character.
fn plain(text: &str) -> String {
    text.lines()
        .map(|l| {
            let l = l.trim_start().trim_start_matches('#').trim_start();
            let l = ["- ", "* ", "• "].iter().find_map(|b| l.strip_prefix(b)).unwrap_or(l);
            l.replace("**", "").replace("__", "").replace('`', "")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A job inside an app that ended well: what worked goes in her notes for that app (the steps that went right, in
/// order), so next time she knows the way.
fn learn(asked: &str, steps: &[Step]) {
    const IN_APP: &[&str] = &["click", "type_in", "type_text", "keys", "see", "read_window"];
    if steps.last().is_none_or(|s| s.outcome.status != Status::Ok) {
        return;
    }
    let app = steps.iter().filter(|s| IN_APP.contains(&s.tool.as_str())).find_map(|s| {
        let w = s.args.get("window").or_else(|| s.args.get("target").filter(|_| s.tool != "click")).and_then(serde_json::Value::as_str)?;
        (!w.trim().is_empty() && w != "front").then(|| w.to_string())
    });
    let Some(app) = app else { return };
    let worked: Vec<String> = steps
        .iter()
        .filter(|s| s.outcome.status == Status::Ok)
        .map(|s| {
            let args: String = s.args.to_string().chars().take(120).collect();
            format!("{} {args}", s.tool)
        })
        .collect();
    let how: String = format!("\"{}\" worked: {}", asked.trim(), worked.join(" → ")).chars().take(500).collect();
    if let Err(e) = capri_core::notes::add(&app, &how) {
        tracing::warn!("notes: {e:#}");
    }
}

fn summarize_steps(steps: &[Step]) -> String {
    if steps.is_empty() {
        return "nothing".into();
    }
    steps
        .iter()
        .map(|s| {
            let st = match s.outcome.status {
                Status::Ok => "ok",
                Status::Failed => "failed",
                Status::Unsure => "unsure",
            };
            let summary: String = s.outcome.summary.chars().take(420).collect();
            format!("{}({}) {st}: {summary}", s.tool, s.args)
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// What her words are built from. The tool results come first and win: the model's own summary can drop or bend a
/// fact ("rain stops at 11" when it isn't raining yet), the tools' reports don't.
fn facts(run: &Run) -> String {
    // Whole results (a long one's start): cut at 420 characters, a listing of five files reached her words as three.
    let steps = run
        .steps
        .iter()
        .map(|s| {
            let st = match s.outcome.status {
                Status::Ok => "ok",
                Status::Failed => "failed",
                Status::Unsure => "unsure",
            };
            format!("{}({}) {st}: {}", s.tool, s.args, s.outcome.summary.chars().take(2500).collect::<String>())
        })
        .collect::<Vec<_>>()
        .join("; ");
    let steps = if steps.is_empty() { "nothing".to_string() } else { steps };
    match (&run.trouble, run.summary.is_empty()) {
        (Some(t), _) => format!("Tool results: {steps}. It didn't get done: {t}."),
        (None, true) => format!("Tool results: {steps}."),
        (None, false) => format!("Tool results: {steps}.\nA first draft of the answer (trust the tool results over it: what a click or play step names is what happened, never another thing seen on the page): {}", run.summary),
    }
}

#[cfg(test)]
mod tests {
    use super::small_talk;

    #[test]
    fn try_again_is_the_last_job_again() {
        use super::retry;
        for s in ["Try again.", "Capri, try again", "come on, try again!", "keep going", "Again.", "do it again please", "don't give up"] {
            assert!(retry(s), "{s}");
        }
        for s in ["Try the other one", "open steam again and play music from my playlist now", "again what?"] {
            assert!(!retry(s), "{s}");
        }
    }

    #[test]
    fn her_lines_have_no_markdown() {
        let line = super::tidy("Here's what I can do:\n\n- **Apps:** open and close them.\n- `music` on YouTube.\n# More");
        assert_eq!(line, "Here's what I can do: Apps: open and close them. Music on YouTube. More");
    }

    #[test]
    fn only_plain_talk_skips_her_tools() {
        for t in ["Thanks.", "thank you so much", "Capri, how are you?", "Hey", "How's your day going?", "good night"] {
            assert!(small_talk(t), "{t}");
        }
        for t in ["thanks, now open discord", "thanks for opening it, close it", "what's the weather", "hey open steam", "how are you doing on that download", "what are you doing"] {
            assert!(!small_talk(t), "{t}");
        }
    }

    #[test]
    fn blackwall_by_voice() {
        assert_eq!(super::blackwall("Capri, Blackwall mode on."), Some(true));
        assert_eq!(super::blackwall("turn off blackwall mode please"), Some(false));
        assert_eq!(super::blackwall("what is the blackwall"), None);
    }

    #[test]
    fn yours() {
        assert_eq!(super::call_me("Call me Max.").as_deref(), Some("Max"));
        assert_eq!(super::call_me("capri, my name is dee").as_deref(), Some("Dee"));
        assert_eq!(super::call_me("call me later"), None);
        assert_eq!(super::call_me("tell him to call me tomorrow"), None);
        assert_eq!(super::remember_that("Remember that I hate mornings.").as_deref(), Some("I hate mornings"));
        assert_eq!(super::remember_that("remember to drink water"), None);
        assert_eq!(super::remember_that("remember that song we heard?"), None);
        assert!(super::is_sorry("Sorry Capri"));
        assert!(super::is_sorry("I'm sorry, I won't throw you again"));
    }

    #[test]
    fn yes_and_no() {
        assert_eq!(super::yes_no("Yes."), Some(true));
        assert_eq!(super::yes_no("Yeah, please."), Some(true));
        assert_eq!(super::yes_no("Nope."), Some(false));
        assert_eq!(super::yes_no("Open Steam."), None);
        assert_eq!(super::yes_no("Yes, yes."), Some(true));
        assert_eq!(super::yes_no("Yeah yeah yeah"), Some(true));
        assert_eq!(super::yes_no("Yes, do it."), Some(true));
        assert_eq!(super::yes_no("Yeah, that's the one."), Some(true));
        assert_eq!(super::yes_no("No, no, not that one."), Some(false));
        assert_eq!(super::yes_no("Yes, and then play the first video."), None);
    }

    #[test]
    fn requests_to_act() {
        use super::{asks_to_do, hard_job};
        for t in ["You need to install it.", "You are supposed to click it, not me.", "pull up discord", "fire up Steam", "Click on the fifth video in my browser.", "Open Streamio for fuck's sake.", "Capri, can you open my Steam and tell me if my download has finished?", "No, no, no. Go back one step in Streo.", "Press on the search bar in stemio.", "Type Jujutsu Kaisen.", "Check if I'm downloading anything on Steam", "cancel the shutdown", "lock my pc", "I'd like you to play the sixth video.", "I want you to go in loaded.com and then search for the cheapest GTA 5 key."] {
            assert!(asks_to_do(t), "{t}");
        }
        for t in ["How big is the moon?", "Yes, yes.", "What time is it?", "I love you", "Am I downloading anything?", "Who made you?", "stop being mean", "sleep tight, Capri", "read me a poem"] {
            assert!(!asks_to_do(t), "{t}");
        }
        assert!(hard_job("Click on the fifth video in my browser."));
        assert!(hard_job("Open Stremio and search Jujutsu Kaisen"));
        assert!(hard_job("Go back one step in Stremio"));
        // Multi-step / site / in-app: must leave the 8B (today's failures).
        assert!(hard_job("I want you to go in loaded.com and then search for the cheapest GTA 5 key."));
        assert!(hard_job("Go to example.com and then search for widgets"));
        assert!(hard_job("Search for cyberpunk age runners in Streamio."));
        assert!(hard_job("Search for cyberpunk edgerunners in Stremio"));
        assert!(hard_job("find the library tab in Epic"));
        assert!(hard_job("Play Young Sheldon on Stremio"));
        assert!(hard_job("play me Young Sheldon on Stremio"));
        assert!(asks_to_do("I want to listen to the latest Skepta album."));
        assert!(hard_job("I want to listen to the latest Skepta album."));
        assert!(hard_job("play the newest Central Cee album"));
        assert!(!hard_job("Open Stremio"));
        assert!(!hard_job("Pause the music"));
        assert!(!hard_job("How big is the moon?"));
        assert!(!hard_job("play lofi on youtube"));
        assert!(!hard_job("play Blinding Lights"));
        assert!(!hard_job("search for cats"));
        assert!(!hard_job("search for cats on google"));
        // Plain open / play still asks_to_do (smart plans them when set); hard_job stays for the note.
        assert!(asks_to_do("Open Stremio"));
        assert!(asks_to_do("play lofi on youtube"));
        assert!(asks_to_do("Switch to Discord"));
    }

    #[test]
    fn other_screen_is_a_move() {
        for t in ["Go to my other screen.", "capri go to the other screen", "switch to my other monitor please"] {
            assert!(super::wants_other_screen(t), "{t}");
        }
        for t in ["go to discord", "other screen is better", "open my other screen recorder"] {
            assert!(!super::wants_other_screen(t), "{t}");
        }
    }

    #[test]
    fn essay_speak_is_caught() {
        assert!(super::looks_like_essay("First I opened the app. Then I searched. After that I clicked play. Finally it worked."));
        assert!(!super::looks_like_essay("Playing Blinding Lights."));
    }

    #[test]
    fn a_parroted_line_is_caught() {
        let before = "V, you threw me around the screen like a glitchin' neon frisbee. Three times. Now I'm mad.";
        assert!(super::repeats("V, you know what? That's fine. But you threw me around the screen like a glitchin' neon frisbee three times.", before));
        assert!(!super::repeats("Doing alright. Bit dizzy still. You?", before));
        assert!(!super::is_sorry("sorry, what time is it"));
    }

    #[test]
    fn forgetting() {
        for t in ["Forget our conversation.", "please forget what we talked about", "forget everything"] {
            assert!(super::forgets(t), "{t}");
        }
        for t in ["forget it", "forget the reminder", "don't forget the milk"] {
            assert!(!super::forgets(t), "{t}");
        }
    }
}
