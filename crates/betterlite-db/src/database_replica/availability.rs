use std::sync::atomic::AtomicBool;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadTarget {
    Reader,
    Writer,
}

// each side's flag is independent: a degraded reader never changes where writes go
#[derive(Debug, Default)]
pub struct Availability {
    pub reader_unavailable: AtomicBool,
    pub writer_unavailable: AtomicBool,
}
