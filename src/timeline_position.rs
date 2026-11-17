//! KSG `timelineScrollAnchoring.ts`: client-local, server-scoped, 100-entry visit cache.
use rune::{Cx, ScrollPos};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug)]
pub struct Position {
    pub row: String,
    pub entry_id: Option<String>,
    pub offset: f32,
    pub scroll: f32,
    pub at_end: bool,
}

pub struct Visit {
    pub scroll: Mutex<ScrollPos>,
    rows: Mutex<HashMap<String, Option<String>>>,
    pub restore: Mutex<Option<Position>>,
}

impl Default for Visit {
    fn default() -> Self {
        Self {
            scroll: Mutex::default(),
            rows: Mutex::default(),
            restore: Mutex::default(),
        }
    }
}

impl Visit {
    pub fn row(&self, key: &str, entry_id: Option<&str>) {
        self.rows
            .lock()
            .unwrap()
            .insert(key.into(), entry_id.map(str::to_owned));
    }

    fn position(&self, thread: usize, cx: &Cx<crate::model::State>) -> Option<Position> {
        // Do not replace a saved position with an intermediate restoration frame.
        if let Some(position) = self.restore.lock().unwrap().as_ref() {
            return Some(position.clone());
        }
        let viewport = cx.window_layout(&format!("conversation-{thread}"))?;
        let rows = self.rows.lock().unwrap();
        let (key, entry_id, rect) = rows
            .iter()
            .filter_map(|(key, entry_id)| {
                let rect = cx.window_layout(key)?;
                (rect[3] > 0.0
                    && rect[1] + rect[3] > viewport[1]
                    && rect[1] < viewport[1] + viewport[3])
                    .then_some((key, entry_id, rect))
            })
            .min_by(|a, b| a.2[1].total_cmp(&b.2[1]))?;
        let scroll = *self.scroll.lock().unwrap();
        Some(Position {
            row: key.clone(),
            entry_id: entry_id.clone(),
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
    pub(crate) fn current(&self, server: &str, thread: usize) -> Option<Arc<Visit>> {
        self.active
            .as_ref()
            .and_then(|((active_server, active_thread), visit)| {
                (active_server == server && *active_thread == thread).then(|| visit.clone())
            })
    }

    pub fn visit(&self) -> Arc<Visit> {
        self.active.as_ref().unwrap().1.clone()
    }

    pub fn open(
        &mut self,
        server: &str,
        thread: usize,
        cx: &Cx<crate::model::State>,
        unloaded: bool,
    ) -> Arc<Visit> {
        let key = (server.to_owned(), thread);
        if let Some((current, visit)) = &self.active
            && current == &key
        {
            // Capture before an unloaded frame unmounts the reader's row.
            // Retain keys/coordinates only, never the transcript payload.
            let position = visit.position(thread, cx);
            if unloaded {
                let mut restore = visit.restore.lock().unwrap();
                if restore.is_none() {
                    *restore = position;
                }
            }
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
