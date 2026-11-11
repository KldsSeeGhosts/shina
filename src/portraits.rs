//! Plays the captains' animation strips in the sidebar without view commits.
//!
//! Each (portrait, clip) pair has one shared `rune::Pixels` stage. The sidebar binds a row's
//! portrait to its stage, and `tick`, called from the app's poll, swaps the stage's frame
//! with `Pixels::set`. Rune then repaints without re-running the view or laying out. `tick`
//! returns when the next frame is due, for `Tx::wake_after`, so nothing wakes between frames
//! and nothing at all wakes while no stage plays.
//!
//! A stage plays only while a sidebar row uses it, that row is on screen (`El::on_visible`),
//! the window has focus (`App::on_focus`, which also covers minimizing) and the system does
//! not ask for reduced motion. Otherwise it shows frame 0, the static face.
use crate::visuals::{Artwork, CAPTAIN_NAMES, Clip, Face};
use rune::Pixels;
use std::{
    cell::Cell,
    collections::{HashMap, HashSet},
    sync::Arc,
    time::{Duration, Instant},
};

struct Stage {
    pixels: Pixels,
    shown: usize,
    started: Option<Instant>,
}

pub struct Portraits {
    artwork: Arc<Artwork>,
    stages: HashMap<(usize, Clip), Stage>,
    /// Sidebar rows (by thread id) that Rune last reported on screen.
    visible: HashSet<usize>,
    focused: bool,
    /// Written by the view, which is the only place that sees the system setting.
    reduced: Cell<bool>,
}

impl Portraits {
    pub fn new(artwork: Arc<Artwork>) -> Self {
        let mut stages = HashMap::new();
        for portrait in 0..CAPTAIN_NAMES.len() {
            let Some(motion) = artwork.motion(portrait) else {
                continue;
            };
            for clip in Clip::ALL {
                let strip = &motion[clip.index()];
                let pixels = Pixels::new();
                pixels.set(strip.size, strip.size, strip.frames[0].clone());
                stages.insert(
                    (portrait, clip),
                    Stage {
                        pixels,
                        shown: 0,
                        started: None,
                    },
                );
            }
        }
        Self {
            artwork,
            stages,
            visible: HashSet::new(),
            focused: true,
            reduced: Cell::new(false),
        }
    }

    /// The stage a portrait shows for `face`, or None to draw the static face.
    pub fn pixels(&self, portrait: usize, face: Face) -> Option<&Pixels> {
        if self.reduced.get() {
            return None;
        }
        let clip = Clip::for_face(face)?;
        self.stages
            .get(&(portrait, clip))
            .map(|stage| &stage.pixels)
    }

    pub fn set_visible(&mut self, row: usize, on: bool) {
        if on {
            self.visible.insert(row);
        } else {
            self.visible.remove(&row);
        }
    }

    pub fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
    }

    pub fn set_reduced_motion(&self, reduced: bool) {
        self.reduced.set(reduced);
    }

    /// Advances every stage to `now` for the sidebar's portraits, given as (row, portrait,
    /// face). Returns how long until the next frame is due, or None when nothing plays.
    pub fn tick(
        &mut self,
        now: Instant,
        rows: impl IntoIterator<Item = (usize, usize, Face)>,
    ) -> Option<Duration> {
        let wanted: HashSet<(usize, Clip)> = rows
            .into_iter()
            .filter(|(row, _, _)| self.visible.contains(row))
            .filter_map(|(_, portrait, face)| Some((portrait, Clip::for_face(face)?)))
            .collect();
        let running = self.focused && !self.reduced.get();
        let mut next: Option<Duration> = None;
        for (&(portrait, clip), stage) in &mut self.stages {
            let frame = if !wanted.contains(&(portrait, clip)) {
                // A row that leaves replays one-shot clips when it returns.
                stage.started = None;
                0
            } else if !running {
                0
            } else {
                let started = *stage.started.get_or_insert(now);
                // Stagger looping clips by portrait so captains don't blink in unison.
                let offset = if clip.repeats() {
                    portrait as u64 * 1300
                } else {
                    0
                };
                let elapsed = now.duration_since(started).as_millis() as u64 + offset;
                let (frame, wait) = position(clip, elapsed);
                if let Some(wait) = wait {
                    next = Some(next.map_or(wait, |n| n.min(wait)));
                }
                frame
            };
            if frame != stage.shown {
                let strip = &self.artwork.motion(portrait).expect("staged motion")[clip.index()];
                stage
                    .pixels
                    .set(strip.size, strip.size, strip.frames[frame].clone());
                stage.shown = frame;
            }
        }
        next
    }

    #[cfg(test)]
    fn shown(&self, portrait: usize, clip: Clip) -> usize {
        self.stages[&(portrait, clip)].shown
    }
}

/// The frame `elapsed` ms into a clip, and how long it stays. A finished one-shot clip rests
/// on frame 0 with nothing more to wait for.
fn position(clip: Clip, elapsed: u64) -> (usize, Option<Duration>) {
    let steps = clip.timeline();
    let total: u64 = steps.iter().map(|(_, ms)| *ms as u64).sum();
    let at = if clip.repeats() {
        elapsed % total
    } else if elapsed >= total {
        return (0, None);
    } else {
        elapsed
    };
    let mut end = 0;
    for (frame, ms) in steps {
        end += *ms as u64;
        if at < end {
            return (*frame, Some(Duration::from_millis(end - at)));
        }
    }
    (0, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn portraits() -> Portraits {
        Portraits::new(Artwork::load(std::path::Path::new("")))
    }

    /// Steps the clock frame by frame, as the poll would, and counts the wakes.
    fn run(p: &mut Portraits, rows: &[(usize, usize, Face)], seconds: u64) -> usize {
        let start = Instant::now();
        let end = start + Duration::from_secs(seconds);
        let mut now = start;
        let mut wakes = 0;
        while let Some(wait) = p.tick(now, rows.iter().copied()) {
            now += wait;
            if now >= end {
                break;
            }
            wakes += 1;
        }
        wakes
    }

    #[test]
    fn every_embedded_captain_has_all_clips_and_frame_zero_is_the_static_face() {
        let art = Artwork::load(std::path::Path::new(""));
        for portrait in 0..CAPTAIN_NAMES.len() {
            let motion = art.motion(portrait).expect("embedded strips");
            for clip in Clip::ALL {
                let strip = &motion[clip.index()];
                assert_eq!(strip.frames.len(), clip.frames());
                assert!(clip.timeline().iter().all(|(f, _)| *f < clip.frames()));
            }
        }
    }

    #[test]
    fn nothing_wakes_without_a_visible_focused_animated_row() {
        let mut p = portraits();
        // Mounted but never reported on screen.
        assert_eq!(p.tick(Instant::now(), [(7, 0, Face::Normal)]), None);
        p.set_visible(7, true);
        // Failed has no clip.
        assert_eq!(p.tick(Instant::now(), [(7, 0, Face::Dizzy)]), None);
        assert!(p.tick(Instant::now(), [(7, 0, Face::Normal)]).is_some());
        p.set_focused(false);
        assert_eq!(p.tick(Instant::now(), [(7, 0, Face::Normal)]), None);
        p.set_focused(true);
        p.set_reduced_motion(true);
        assert_eq!(p.tick(Instant::now(), [(7, 0, Face::Normal)]), None);
        assert!(
            p.pixels(0, Face::Normal).is_none(),
            "reduced motion draws static faces"
        );
        p.set_reduced_motion(false);
        p.set_visible(7, false);
        assert_eq!(p.tick(Instant::now(), [(7, 0, Face::Normal)]), None);
    }

    #[test]
    fn idle_blinks_on_a_schedule_and_pausing_rests_on_the_static_face() {
        let mut p = portraits();
        p.set_visible(1, true);
        let rows = [(1, 0, Face::Normal)];
        // About a frame a second: breathing and one blink per seven-second loop.
        let wakes = run(&mut p, &rows, 21);
        assert!((18..=30).contains(&wakes), "{wakes} idle wakes in 21 s");
        let start = Instant::now();
        p.tick(start, rows);
        p.tick(start + Duration::from_millis(1800), rows);
        assert_ne!(p.shown(0, Clip::Idle), 0, "mid-breath");
        p.set_focused(false);
        p.tick(start + Duration::from_millis(1900), rows);
        assert_eq!(p.shown(0, Clip::Idle), 0);
    }

    #[test]
    fn working_is_livelier_than_idle_and_happy_plays_once() {
        let mut p = portraits();
        p.set_visible(1, true);
        let working = run(&mut p, &[(1, 1, Face::Determined)], 10);
        assert!(working >= 30, "{working} working wakes in 10 s");
        let mut p = portraits();
        p.set_visible(1, true);
        let start = Instant::now();
        assert!(p.tick(start, [(1, 2, Face::Happy)]).is_some());
        assert_eq!(
            p.tick(start + Duration::from_secs(1), [(1, 2, Face::Happy)]),
            None
        );
        assert_eq!(p.shown(2, Clip::Happy), 0);
        // Leaving and returning replays it.
        p.tick(start + Duration::from_secs(2), []);
        assert!(
            p.tick(start + Duration::from_secs(3), [(1, 2, Face::Happy)])
                .is_some()
        );
    }

    #[test]
    fn one_wake_serves_every_row_that_shares_a_stage() {
        let mut p = portraits();
        for row in 1..=3 {
            p.set_visible(row, true);
        }
        let one = run(&mut p, &[(1, 0, Face::Determined)], 10);
        let mut p2 = portraits();
        for row in 1..=3 {
            p2.set_visible(row, true);
        }
        let three = run(
            &mut p2,
            &[
                (1, 0, Face::Determined),
                (2, 0, Face::Determined),
                (3, 0, Face::Determined),
            ],
            10,
        );
        assert_eq!(one, three);
    }

    #[test]
    fn a_drop_in_set_without_strips_stays_static() {
        let root = std::env::temp_dir().join(format!("shina-static-set-{}", std::process::id()));
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/crew/ember");
        let folder = root.join("assets/crew/ember");
        std::fs::create_dir_all(&folder).unwrap();
        for face in ["Normal", "Happy", "Determined", "Surprised", "Dizzy", "Sad"] {
            std::fs::copy(
                source.join(format!("{face}.png")),
                folder.join(format!("{face}.png")),
            )
            .unwrap();
        }
        let art = Artwork::load(&root);
        assert!(art.motion(0).is_none(), "the drop-in Ember has no strips");
        assert!(art.motion(1).is_some(), "the embedded Juniper keeps hers");
        for portrait in 4..CAPTAIN_NAMES.len() {
            assert!(
                art.motion(portrait).is_some(),
                "captain {portrait} has no drop-in folder and keeps the embedded art"
            );
        }
        let mut p = Portraits::new(art);
        p.set_visible(1, true);
        assert!(p.pixels(0, Face::Normal).is_none());
        assert_eq!(p.tick(Instant::now(), [(1, 0, Face::Normal)]), None);
        std::fs::remove_dir_all(&root).ok();
    }
}
