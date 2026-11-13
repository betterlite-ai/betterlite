use std::sync::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadTarget {
    Reader,
    Writer,
}

// each side's flag is independent: a degraded reader never changes where writes go
#[derive(Debug, Default)]
pub struct Availability {
    reader_unavailable: Mutex<bool>,
    writer_unavailable: Mutex<bool>,
}

impl Availability {
    #[must_use]
    pub fn reader_unavailable(&self) -> bool {
        match self.reader_unavailable.lock() {
            Ok(unavailable) => *unavailable,
            Err(poisoned) => *poisoned.into_inner(),
        }
    }

    #[must_use]
    pub fn writer_unavailable(&self) -> bool {
        match self.writer_unavailable.lock() {
            Ok(unavailable) => *unavailable,
            Err(poisoned) => *poisoned.into_inner(),
        }
    }

    pub fn mark_reader_unavailable(&self) {
        match self.reader_unavailable.lock() {
            Ok(mut unavailable) => *unavailable = true,
            Err(poisoned) => *poisoned.into_inner() = true,
        }
    }

    pub fn clear_reader_unavailable(&self) {
        match self.reader_unavailable.lock() {
            Ok(mut unavailable) => *unavailable = false,
            Err(poisoned) => *poisoned.into_inner() = false,
        }
    }

    pub fn mark_writer_unavailable(&self) {
        match self.writer_unavailable.lock() {
            Ok(mut unavailable) => *unavailable = true,
            Err(poisoned) => *poisoned.into_inner() = true,
        }
    }

    // an external health probe owns proof of recovery, so this only clears the flag it proved
    pub fn mark_writer_recovered(&self) {
        match self.writer_unavailable.lock() {
            Ok(mut unavailable) => *unavailable = false,
            Err(poisoned) => *poisoned.into_inner() = false,
        }
    }
}
