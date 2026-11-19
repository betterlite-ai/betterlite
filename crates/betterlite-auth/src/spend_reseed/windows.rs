use betterlite_db::budget_permission_rows::Timestamp;

use crate::budget_counter::CounterEntityType;

// the window read methods live in reseed.rs: clippy allows one inherent impl block per type
pub struct WindowQuery<'entity> {
    pub entity_type: &'entity CounterEntityType,
    pub entity_id: &'entity str,
    pub start: Timestamp,
}
