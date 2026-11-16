pub mod aggregation;
pub mod fields;
pub mod metrics;
pub mod overview;
pub mod records;
pub mod response;
pub mod status;
pub mod window;

use std::collections::BTreeMap;

// LiteLLM refuses a dashboard window longer than a year and a day
pub const USAGE_MAX_RANGE_DAYS: i64 = 366;

// the counters a row with no priced share carries; first_match hands it out for a missing key
pub static EMPTY_UNITS: BTreeMap<String, i64> = BTreeMap::new();
