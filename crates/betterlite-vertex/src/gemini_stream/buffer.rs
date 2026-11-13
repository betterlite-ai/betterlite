use serde_json::value::RawValue;

use super::error::StreamError;

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
        let mut size = self
            .text
            .len()
            .saturating_sub(self.offset)
            .saturating_add(fragment.len());
        for pending in &self.fragments {
            size = size.saturating_add(pending.len());
        }
        if size > MAX_RETAINED_BYTES {
            return Err(StreamError::TooLarge { size });
        }
        self.fragments.push(fragment.to_owned());
        match fragment.trim_end().chars().next_back() {
            Some('}' | ']') => self.could_close = true,
            Some(_) => self.could_close = false,
            None => {}
        }
        Ok(())
    }

    pub(super) fn has_buffered_content(&self) -> bool {
        if !self.fragments.is_empty() {
            return true;
        }
        self.offset < self.text.len()
    }

    pub(super) fn pop_next_value(&mut self) -> Option<Box<RawValue>> {
        self.materialize();
        let rest = self.text.get(self.offset..)?;
        let trimmed = rest.trim_start();
        if trimmed.is_empty() {
            self.offset = self.text.len();
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
        self.offset = 0;
        let joined = self.fragments.concat();
        self.copied = self
            .copied
            .saturating_add(rebuilt.len())
            .saturating_add(joined.len());
        rebuilt.push_str(&joined);
        self.text = rebuilt;
        self.fragments.clear();
    }
}
