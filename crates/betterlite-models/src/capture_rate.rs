pub mod alert;
pub mod compute;
pub mod format;
pub mod report;
pub mod settings;

use rust_decimal::Decimal;

mod tests;

pub const CHECK_JOB_ID: &str = "spend_capture_rate_check_job";
pub const CHECK_LOCK_TTL_SECONDS: i64 = 900;
pub const MAX_RANGE_DAYS: i64 = 180;
pub const DOCS_URL: &str = "https://docs.litellm.ai/docs/proxy/spend_capture_rate";
pub const OPENAI_BILLED_PROVIDERS: [&str; 2] = ["openai", "text-completion-openai"];
pub const DEFAULT_THRESHOLD: Decimal = Decimal::from_parts(9, 0, 0, false, 1);
pub const DEFAULT_LOOKBACK_DAYS: i64 = 7;
