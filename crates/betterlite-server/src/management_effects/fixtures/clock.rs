use chrono::{DateTime, FixedOffset, Utc};

use crate::failure_hook::ports::Clock;

use super::journal::Journal;

pub(crate) const START_INSTANT: &str = "2024-01-01T00:00:00+00:00";
pub(crate) const END_INSTANT: &str = "2024-01-01T00:00:02+00:00";

// Each wrapper run samples twice, start then end, so even samples are starts and odd ones ends
#[derive(Default)]
pub(crate) struct ScriptedClock {
    pub(crate) samples: Journal<DateTime<FixedOffset>>,
}

impl Clock for ScriptedClock {
    fn now(&self) -> DateTime<FixedOffset> {
        let text = match self.samples.entries().len() {
            0 | 2 | 4 | 6 => START_INSTANT,
            _ => END_INSTANT,
        };
        let instant = DateTime::parse_from_rfc3339(text).expect("the fixture instant parses");
        self.samples.record(instant);
        instant
    }
}

pub(crate) fn start_instant() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(START_INSTANT)
        .expect("the fixture start instant parses")
        .with_timezone(&Utc)
}

pub(crate) fn end_instant() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(END_INSTANT)
        .expect("the fixture end instant parses")
        .with_timezone(&Utc)
}
