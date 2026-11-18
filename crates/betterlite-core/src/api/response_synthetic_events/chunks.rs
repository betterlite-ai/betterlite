use super::error::SyntheticEventsError;

pub const MAX_SYNTHETIC_EVENTS: usize = 1_000_000;

pub const MAX_SYNTHETIC_BYTES: usize = 256 * 1024 * 1024;

#[derive(Default)]
pub struct EventBudget {
    events: usize,
    bytes: usize,
}

impl EventBudget {
    pub fn charge(&mut self, events: usize, bytes: usize) -> Result<(), SyntheticEventsError> {
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
    budget.charge(
        count,
        count.saturating_mul(item_id_bytes.saturating_add(text.len())),
    )?;
    Ok(characters
        .chunks(size)
        .map(|chunk| chunk.iter().collect::<String>())
        .collect())
}
