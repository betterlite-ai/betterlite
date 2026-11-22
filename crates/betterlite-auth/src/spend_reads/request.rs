use betterlite_db::budget_permission_rows::Timestamp;
use rust_decimal::Decimal;

use crate::budget_counter::CounterEntityType;

// the caller facts get_current_spend takes, for a counter and optionally its window
pub struct SpendReadRequest<'request> {
    pub counter_key: &'request str,
    pub fallback_spend: Decimal,
    pub max_budget: Option<Decimal>,
    pub window: Option<SpendWindow<'request>>,
    pub fallback_authoritative: bool,
}

// LiteLLM's window_entity_type, window_entity_id, window_duration and window_start
#[derive(Clone, Copy, Debug)]
pub struct SpendWindow<'window> {
    pub entity_type: &'window CounterEntityType,
    pub entity_id: &'window str,
    pub window_duration: Option<&'window str>,
    pub window_start: Timestamp,
}
