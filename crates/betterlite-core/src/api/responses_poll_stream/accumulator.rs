use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::api::responses_poll_state::record::PollPatch;

use super::UPDATE_INTERVAL_MILLIS;
use super::clock::PollClock;
use super::event::{Item, PollEvent, PollEventError, TERMINAL_FIELDS, Terminal};

#[derive(Debug, Default)]
pub struct OutputAccumulator {
    output_items: IndexMap<String, IndexMap<String, Json>>,
    accumulated_text: IndexMap<(String, i64), String>,
    terminal: Option<TerminalRecord>,
    dirty: bool,
    last_update_millis: u64,
}

#[derive(Debug)]
struct TerminalRecord {
    status: String,
    fields: IndexMap<String, Json>,
    error: Json,
}

impl Default for TerminalRecord {
    fn default() -> Self {
        Self {
            status: String::new(),
            fields: IndexMap::new(),
            error: Json::Null,
        }
    }
}

impl OutputAccumulator {
    // the source reads the clock once before the loop and once per event after it
    #[must_use]
    pub fn started(clock: &dyn PollClock) -> Self {
        Self {
            last_update_millis: clock.now_millis(),
            ..Self::default()
        }
    }

    pub fn apply(&mut self, event: PollEvent) -> Result<(), PollEventError> {
        match event {
            PollEvent::OutputItemAdded(item) | PollEvent::OutputItemDone(item) => {
                self.store_item(item);
            }
            PollEvent::ContentPartAdded { item_id, part } => {
                if let Some(id) = item_id
                    && let Some(item) = self.output_items.get_mut(&id)
                {
                    let mut parts = content_list(item);
                    parts.push(part);
                    item.insert(String::from("content"), Json::List(parts));
                    self.dirty = true;
                }
            }
            PollEvent::ContentPartDone {
                item_id,
                part,
                content_index,
            } => {
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
            PollEvent::OutputTextDelta {
                item_id,
                content_index,
                delta,
            } => {
                let Some(id) = item_id else {
                    return Ok(());
                };
                let Some(item) = self.output_items.get_mut(&id) else {
                    return Ok(());
                };
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
            PollEvent::Terminal(terminal) => self.apply_terminal(terminal),
            PollEvent::InProgress | PollEvent::Unknown => {}
        }
        Ok(())
    }

    // reads the clock even when nothing is dirty, exactly like the source's nested flush
    pub fn flush_if_due(&mut self, clock: &dyn PollClock) -> Option<Json> {
        let now = clock.now_millis();
        if !self.dirty || now.saturating_sub(self.last_update_millis) < UPDATE_INTERVAL_MILLIS {
            return None;
        }
        Some(self.flushed(now))
    }

    pub fn flush_forced(&mut self, clock: &dyn PollClock) -> Option<Json> {
        let now = clock.now_millis();
        if !self.dirty {
            return None;
        }
        Some(self.flushed(now))
    }

    #[must_use]
    pub fn final_patch(&self) -> PollPatch {
        let mut fields = IndexMap::with_capacity(TERMINAL_FIELDS.len() + 2);
        fields.insert(
            String::from("status"),
            Json::Text(self.final_status().to_owned()),
        );
        let terminal = self.terminal.as_ref();
        for name in TERMINAL_FIELDS {
            let value = terminal
                .and_then(|record| record.fields.get(name))
                .cloned()
                .unwrap_or(Json::Null);
            fields.insert(name.to_owned(), value);
        }
        fields.insert(
            String::from("error"),
            terminal.map_or(Json::Null, |record| record.error.clone()),
        );
        PollPatch(fields)
    }

    // the source falls back to "completed" when no terminal event ever set a status
    #[must_use]
    pub fn final_status(&self) -> &str {
        self.terminal
            .as_ref()
            .map_or("completed", |record| record.status.as_str())
    }

    fn store_item(&mut self, item: Item) {
        let Some(id) = item.id else {
            return;
        };
        self.output_items.insert(id, item.value);
        self.dirty = true;
    }

    fn apply_terminal(&mut self, terminal: Terminal) {
        // an absent response.status leaves the status to the event type
        let status = terminal
            .status
            .unwrap_or_else(|| terminal.kind.default_status().to_owned());
        let record = self.terminal.get_or_insert_with(TerminalRecord::default);
        record.status = status;
        record.fields = terminal.fields;
        // only a failed or incomplete event names an error, and an absent one clears the previous
        if let Some(error) = terminal.error {
            record.error = error;
        }
        if let Some(items) = terminal.output {
            for item in items {
                if let Some(id) = item.id {
                    self.output_items.insert(id, item.value);
                }
            }
            self.dirty = true;
        }
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

fn content_list(item: &IndexMap<String, Json>) -> Vec<Json> {
    let Some(Json::List(parts)) = item.get("content") else {
        return Vec::new();
    };
    parts.clone()
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
    Ok(usize::try_from(index)
        .ok()
        .filter(|position| *position < len))
}
