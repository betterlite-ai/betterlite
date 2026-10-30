use std::time::Duration;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TimeBound {
    Finite(Duration),
    Infinite,
}

pub const LATENCY_BUCKETS: [TimeBound; 18] = [
    TimeBound::Finite(Duration::from_millis(5)),
    TimeBound::Finite(Duration::from_millis(10)),
    TimeBound::Finite(Duration::from_millis(25)),
    TimeBound::Finite(Duration::from_millis(50)),
    TimeBound::Finite(Duration::from_millis(100)),
    TimeBound::Finite(Duration::from_millis(250)),
    TimeBound::Finite(Duration::from_millis(500)),
    TimeBound::Finite(Duration::from_secs(1)),
    TimeBound::Finite(Duration::from_secs(2)),
    TimeBound::Finite(Duration::from_secs(5)),
    TimeBound::Finite(Duration::from_secs(10)),
    TimeBound::Finite(Duration::from_secs(30)),
    TimeBound::Finite(Duration::from_mins(1)),
    TimeBound::Finite(Duration::from_mins(2)),
    TimeBound::Finite(Duration::from_mins(5)),
    TimeBound::Finite(Duration::from_mins(7)),
    TimeBound::Finite(Duration::from_mins(10)),
    TimeBound::Infinite,
];

pub const BATCH_DURATION_BUCKETS: [TimeBound; 13] = [
    TimeBound::Finite(Duration::from_mins(1)),
    TimeBound::Finite(Duration::from_mins(2)),
    TimeBound::Finite(Duration::from_mins(5)),
    TimeBound::Finite(Duration::from_mins(10)),
    TimeBound::Finite(Duration::from_mins(15)),
    TimeBound::Finite(Duration::from_mins(30)),
    TimeBound::Finite(Duration::from_hours(1)),
    TimeBound::Finite(Duration::from_hours(2)),
    TimeBound::Finite(Duration::from_hours(4)),
    TimeBound::Finite(Duration::from_hours(8)),
    TimeBound::Finite(Duration::from_hours(12)),
    TimeBound::Finite(Duration::from_hours(24)),
    TimeBound::Infinite,
];
