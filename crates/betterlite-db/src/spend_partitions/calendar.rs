use chrono::{Datelike as _, Days, Months, NaiveDate, Utc};

pub const SPEND_LOGS_TABLE: &str = "LiteLLM_SpendLogs";

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PartitionInterval {
    Day,
    Week,
    Month,
}

impl PartitionInterval {
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "day" => Some(Self::Day),
            "week" => Some(Self::Week),
            "month" => Some(Self::Month),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedPartition {
    pub name: String,
    pub lower: NaiveDate,
    pub upper: NaiveDate,
}

// the caller's clock: UTC decides which period is current, and tests pass a fixed date
pub trait PartitionClock: Send + Sync {
    fn today(&self) -> NaiveDate;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct UtcPartitionClock;

impl PartitionClock for UtcPartitionClock {
    fn today(&self) -> NaiveDate {
        Utc::now().date_naive()
    }
}

// period_start: a week starts on Monday and a month on its first day
fn period_start(day: NaiveDate, interval: PartitionInterval) -> Option<NaiveDate> {
    match interval {
        PartitionInterval::Day => Some(day),
        PartitionInterval::Week => {
            let offset = day.weekday().num_days_from_monday();
            day.checked_sub_days(Days::new(u64::from(offset)))
        }
        PartitionInterval::Month => day.with_day(1),
    }
}

const fn next_period_start(start: NaiveDate, interval: PartitionInterval) -> Option<NaiveDate> {
    match interval {
        PartitionInterval::Day => start.checked_add_days(Days::new(1)),
        PartitionInterval::Week => start.checked_add_days(Days::new(7)),
        PartitionInterval::Month => start.checked_add_months(Months::new(1)),
    }
}

// upcoming_partitions: the current period plus the next `ahead` ones, and an empty list for a negative ahead
#[must_use]
pub fn upcoming_partitions(
    today: NaiveDate,
    interval: PartitionInterval,
    ahead: i64,
) -> Vec<PlannedPartition> {
    let mut partitions = Vec::new();
    let Some(mut start) = period_start(today, interval) else {
        return partitions;
    };
    for _ in 0..ahead.saturating_add(1) {
        let Some(upper) = next_period_start(start, interval) else {
            break;
        };
        partitions.push(PlannedPartition {
            name: format!("{SPEND_LOGS_TABLE}_p{}", start.format("%Y%m%d")),
            lower: start,
            upper,
        });
        start = upper;
    }
    partitions
}
