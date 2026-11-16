use chrono::NaiveDate;

// The two shapes B1692 hands back after a run; every field is written where a result is built, since a zero is a real count.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RollupResult {
    pub day: NaiveDate,
    pub models_processed: usize,
    pub rows_written: usize,
    pub rows_failed: usize,
    pub lapsed: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BackfillResult {
    pub start: NaiveDate,
    pub end: NaiveDate,
    pub days_scanned: usize,
    pub rows_written: usize,
    pub rows_failed: usize,
}
