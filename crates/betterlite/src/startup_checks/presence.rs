#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatabasePresence {
    Present,
    Absent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RedisPresence {
    Present,
    Absent,
}
