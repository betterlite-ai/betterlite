use super::error::StreamError;
use serde_json::value::RawValue;
// the pooled transport caps one reply at 16 MiB, so a longer unconsumed fragment is malformed input
pub(super) const MAX_RETAINED_BYTES: usize = 0x0100_0000;
#[derive(Debug, Default)]
pub(super) struct FragmentBuffer {
    pub(super) fragments: Vec<String>,
    pub(super) text: String,
    pub(super) offset: usize,
    pub(super) could_close: bool,
    pub(super) copied: usize,
}
impl FragmentBuffer {
    pub(super) fn append(&mut self, fragment: &str) -> Result<(), StreamError> {
        // an empty fragment would make the next pop rebuild the unconsumed tail for nothing
        if fragment.is_empty() {
            return Ok(());
        }
        let retained = self.fragments.iter().fold(
            self.text.len().saturating_sub(self.offset) + fragment.len(),
            |total, pending| total.saturating_add(pending.len()),
        );
        if retained > MAX_RETAINED_BYTES {
            return Err(StreamError::TooLarge { size: retained });
        }
        self.fragments.push(fragment.to_owned());
        match fragment.trim_end().chars().next_back() {
            Some('}' | ']') => self.could_close = true,
            Some(_) => self.could_close = false,
            None => {}
        }
        Ok(())
    }
    pub(super) fn pop_next_value(&mut self) -> Option<Box<RawValue>> {
        self.materialize();
        let trimmed = self.text.get(self.offset..)?.trim_start();
        if trimmed.is_empty() {
            return None;
        }
        let start = self.text.len().checked_sub(trimmed.len())?;
        let mut values = serde_json::Deserializer::from_str(trimmed).into_iter::<Box<RawValue>>();
        let Some(Ok(value)) = values.next() else {
            return None;
        };
        self.offset = start.checked_add(values.byte_offset())?;
        if self.offset >= self.text.len() {
            self.text.clear();
            self.offset = 0;
            self.could_close = false;
        }
        Some(value)
    }
    fn materialize(&mut self) {
        if self.fragments.is_empty() {
            return;
        }
        let mut rebuilt = self.text.get(self.offset..).unwrap_or_default().to_owned();
        let joined = self.fragments.concat();
        self.copied = self
            .copied
            .saturating_add(rebuilt.len())
            .saturating_add(joined.len());
        rebuilt.push_str(&joined);
        self.text = rebuilt;
        self.offset = 0;
        self.fragments.clear();
    }
}
