//! KSG `timelineScrollAnchoring.ts`: client-local, server-scoped, 100-entry visit cache.
use rune::{Cx, ScrollPos};
use std::collections::{HashSet, VecDeque};
use std::sync::{Arc, Mutex, atomic::AtomicBool};

#[derive(Clone, Debug)]
pub struct Position {
    pub row: String,
    pub offset: f32,
    pub scroll: f32,
    pub at_end: bool,
}

pub struct Visit {
    pub scroll: Mutex<ScrollPos>,
    rows: Mutex<HashSet<String>>,
    pub restore: Mutex<Option<Position>>,
    pub opened: AtomicBool,
}

impl Default for Visit {
    fn default() -> Self {
        Self {
            scroll: Mutex::default(),
            rows: Mutex::default(),
            restore: Mutex::default(),
            opened: AtomicBool::new(false),
        }
    }
}

impl Visit {
    pub fn row(&self, key: &str) {
        self.rows.lock().unwrap().insert(key.into());
    }

    fn position(&self, thread: usize, cx: &Cx<crate::model::State>) -> Option<Position> {
        // Do not replace a saved position with an intermediate restoration frame.
        if let Some(position) = self.restore.lock().unwrap().as_ref() {
            return Some(position.clone());
        }
        let viewport = cx.window_layout(&format!("conversation-{thread}"))?;
        let rows = self.rows.lock().unwrap();
        let (key, rect) = rows
            .iter()
            .filter_map(|key| {
                let rect = cx.window_layout(key)?;
                (rect[3] > 0.0
                    && rect[1] + rect[3] > viewport[1]
                    && rect[1] < viewport[1] + viewport[3])
                    .then_some((key, rect))
            })
            .min_by(|a, b| a.1[1].total_cmp(&b.1[1]))?;
        let scroll = *self.scroll.lock().unwrap();
        Some(Position {
            row: key.clone(),
            offset: viewport[1] - rect[1],
            scroll: scroll.offset[1],
            at_end: scroll.max[1] - scroll.offset[1] <= 40.0,
        })
    }
}

#[derive(Default)]
pub struct Positions {
    saved: VecDeque<((String, usize), Position)>,
    active: Option<((String, usize), Arc<Visit>)>,
}

impl Positions {
    pub fn visit(&self) -> Arc<Visit> {
        self.active.as_ref().unwrap().1.clone()
    }

    pub fn open(
        &mut self,
        server: &str,
        thread: usize,
        cx: &Cx<crate::model::State>,
    ) -> Arc<Visit> {
        let key = (server.to_owned(), thread);
        if let Some((current, visit)) = &self.active
            && current == &key
        {
            // Prime Rune's tracked layout reads while this list is mounted.
            let _ = visit.position(thread, cx);
            visit.rows.lock().unwrap().clear();
            return visit.clone();
        }
        if let Some((previous, visit)) = self.active.take()
            && let Some(position) = visit.position(previous.1, cx)
        {
            self.saved.retain(|(key, _)| key != &previous);
            self.saved.push_back((previous, position));
            if self.saved.len() > 100 {
                self.saved.pop_front();
            }
        }
        let position = self
            .saved
            .iter()
            .find(|(saved, _)| saved == &key)
            .map(|(_, position)| position.clone());
        let visit = Arc::new(Visit::default());
        *visit.restore.lock().unwrap() = position;
        self.active = Some((key, visit.clone()));
        visit
    }
}
