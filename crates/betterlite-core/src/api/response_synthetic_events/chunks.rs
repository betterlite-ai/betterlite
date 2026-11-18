use std::io;

use super::error::SyntheticEventsError;

pub const MAX_SYNTHETIC_EVENTS: usize = 1_000_000;

pub const MAX_SYNTHETIC_BYTES: usize = 256 * 1024 * 1024;
const EVENT_OVERHEAD_BYTES: usize = 64;

#[derive(Default)]
pub struct EventBudget {
    events: usize,
    bytes: usize,
}

impl EventBudget {
    pub const fn charge(
        &mut self,
        events: usize,
        bytes: usize,
    ) -> Result<(), SyntheticEventsError> {
        self.events = self.events.saturating_add(events);
        self.bytes = self.bytes.saturating_add(bytes);
        if self.events > MAX_SYNTHETIC_EVENTS || self.bytes > MAX_SYNTHETIC_BYTES {
            return Err(SyntheticEventsError::EventBudgetExceeded);
        }
        Ok(())
    }
}

pub fn delta_chunks(
    text: &str,
    chunk_size: i64,
    budget: &mut EventBudget,
    item_id_bytes: usize,
) -> Result<Vec<String>, SyntheticEventsError> {
    if chunk_size == 0 {
        return Err(SyntheticEventsError::ZeroChunkSize);
    }
    let characters: Vec<char> = text.chars().collect();
    let Ok(size) = usize::try_from(chunk_size) else {
        return Ok(Vec::new());
    };
    let count = characters.len().div_ceil(size);
    let per_event = item_id_bytes.saturating_add(EVENT_OVERHEAD_BYTES);
    budget.charge(
        count,
        text.len().saturating_add(count.saturating_mul(per_event)),
    )?;
    Ok(characters
        .chunks(size)
        .map(|chunk| chunk.iter().collect::<String>())
        .collect())
}

#[derive(Default)]
struct ByteCount(usize);

impl io::Write for ByteCount {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.0 = self.0.saturating_add(data.len());
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[must_use]
pub fn payload_bytes<T>(value: &T) -> usize
where
    T: serde::Serialize,
{
    let mut count = ByteCount::default();
    if serde_json::to_writer(&mut count, value).is_err() {
        // an unmeasurable payload must overcharge the budget, never slip under it
        return usize::MAX;
    }
    count.0
}
