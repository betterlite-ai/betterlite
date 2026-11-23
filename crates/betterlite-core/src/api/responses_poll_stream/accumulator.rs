use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::api::responses_poll_state::record::PollPatch;

use super::UPDATE_INTERVAL;
use super::clock::PollClock;
use super::event::{PollEvent, PollEventError, TERMINAL_FIELDS, Terminal};

#[derive(Debug, Default)]
pub struct OutputAccumulator {
    output_items: IndexMap<String, IndexMap<String, Json>>,
    accumulated_text: IndexMap<(String, i64), String>,
    terminal: Option<Terminal>,
    error: Option<Json>,
    dirty: bool,
    last_update_millis: u64,
}

impl OutputAccumulator {
    // the source reads the clock once before the loop and once per event after it
    #[must_use]
    pub fn started(clock: &dyn PollClock) -> Self {
        Self {
            output_items: IndexMap::new(),
            accumulated_text: IndexMap::new(),
            terminal: None,
            error: None,
            dirty: false,
            last_update_millis: clock.now_millis(),
        }
    }

    pub fn apply(&mut self, event: PollEvent) -> Result<(), PollEventError> {
        match event {
            PollEvent::OutputItemAdded(item) | PollEvent::OutputItemDone(item) => {
                if let Some(id) = item.id {
                    self.output_items.insert(id, item.value);
                    self.dirty = true;
                }
            }
            PollEvent::ContentPartAdded(item_id, part) => {
                if let Some(id) = item_id
                    && let Some(item) = self.output_items.get_mut(&id)
                {
                    let mut parts = match item.get("content") {
                        Some(Json::List(parts)) => parts.clone(),
                        Some(_) | None => Vec::new(),
                    };
                    parts.push(part);
                    item.insert(String::from("content"), Json::List(parts));
                    self.dirty = true;
                }
            }
            PollEvent::ContentPartDone(item_id, part, content_index) => {
                if let Some(id) = item_id
                    && let Some(item) = self.output_items.get_mut(&id)
                {
                    // the source compares enumerate positions, so a negative index replaces nothing
                    if let Ok(position) = usize::try_from(content_index)
                        && let Some(Json::List(parts)) = item.get_mut("content")
                        && let Some(slot) = parts.get_mut(position)
                    {
                        *slot = part;
                    }
                    self.dirty = true;
                }
            }
            PollEvent::OutputTextDelta(item_id, content_index, delta) => {
                if let Some(id) = item_id
                    && let Some(item) = self.output_items.get_mut(&id)
                {
                    let accumulated = self
                        .accumulated_text
                        .entry((id, content_index))
                        .or_default();
                    accumulated.push_str(&delta);
                    let text = accumulated.clone();
                    if let Some(Json::List(parts)) = item.get_mut("content")
                        && let Some(position) = part_index(content_index, parts.len())?
                        && let Some(Json::Object(part)) = parts.get_mut(position)
                    {
                        part.insert(String::from("text"), Json::Text(text));
                    }
                    self.dirty = true;
                }
            }
            PollEvent::Terminal(terminal) => self.apply_terminal(terminal),
            PollEvent::InProgress | PollEvent::Unknown => {}
        }
        Ok(())
    }

    // reads the clock even when nothing is dirty, exactly like the source's nested flush
    pub fn flush_if_due(&mut self, clock: &dyn PollClock) -> Option<Json> {
        let now = clock.now_millis();
        let due = self.dirty && now.saturating_sub(self.last_update_millis) >= UPDATE_INTERVAL;
        due.then(|| self.flushed(now))
    }

    pub fn flush_forced(&mut self, clock: &dyn PollClock) -> Option<Json> {
        let now = clock.now_millis();
        self.dirty.then(|| self.flushed(now))
    }

    #[must_use]
    pub fn final_patch(&self) -> PollPatch {
        let mut fields = IndexMap::with_capacity(TERMINAL_FIELDS.split(' ').count() + 2);
        let status = Json::Text(self.final_status().to_owned());
        fields.insert(String::from("status"), status);
        let terminal = self.terminal.as_ref();
        for name in TERMINAL_FIELDS.split(' ') {
            let value = terminal
                .and_then(|record| record.fields.get(name))
                .cloned()
                .unwrap_or(Json::Null);
            fields.insert(name.to_owned(), value);
        }
        let error = self.error.clone().unwrap_or(Json::Null);
        fields.insert(String::from("error"), error);
        PollPatch(fields)
    }

    // the source falls back to "completed" when no terminal event ever set a status
    #[must_use]
    pub fn final_status(&self) -> &str {
        self.terminal.as_ref().map_or("completed", |terminal| {
            terminal
                .status
                .as_deref()
                .unwrap_or_else(|| terminal.kind.default_status())
        })
    }

    fn apply_terminal(&mut self, terminal: Terminal) {
        // a failed or incomplete event names an error, and an absent one clears the previous
        if let Some(error) = &terminal.error {
            self.error = Some(error.clone());
        }
        if let Some(items) = &terminal.output {
            for item in items {
                if let Some(id) = &item.id {
                    self.output_items.insert(id.clone(), item.value.clone());
                }
            }
            self.dirty = true;
        }
        self.terminal = Some(terminal);
    }

    fn flushed(&mut self, now: u64) -> Json {
        self.dirty = false;
        self.last_update_millis = now;
        Json::List(
            self.output_items
                .values()
                .map(|item| Json::Object(item.clone()))
                .collect(),
        )
    }
}

// python allows a negative index and raises only when it falls before the first part
fn part_index(index: i64, len: usize) -> Result<Option<usize>, PollEventError> {
    if index < 0 {
        let distance = usize::try_from(index.unsigned_abs())
            .map_err(|_error| PollEventError::ContentIndex { index })?;
        let Some(position) = len.checked_sub(distance) else {
            return Err(PollEventError::ContentIndex { index });
        };
        return Ok(Some(position));
    }
    // a positive index past the last part selects nothing, exactly like python's failed bounds check
    Ok(usize::try_from(index).ok())
}
