use crate::daily_spend_sql::entities::{DailySpendEntity, SpendRow};
use crate::daily_spend_sql::values::SpendColumnValue;
use crate::spend_queues::daily::DailySpendUpdate;

use super::DAILY_BATCH_SIZE;

#[must_use]
pub const fn entity_id_field(entity: DailySpendEntity) -> &'static str {
    match entity {
        DailySpendEntity::User => "user_id",
        DailySpendEntity::Team => "team_id",
        DailySpendEntity::Org => "organization_id",
        DailySpendEntity::Tag => "tag",
        DailySpendEntity::EndUser => "end_user_id",
        DailySpendEntity::Agent => "agent_id",
    }
}

// The keys of the next chunk: the source's five sort fields, then insertion order, then at most a hundred entries.
#[must_use]
pub fn ordered_chunk_keys(entity: DailySpendEntity, pending: &DailySpendUpdate) -> Vec<String> {
    let id_field = entity_id_field(entity);
    let mut entries: Vec<(&String, [String; 5])> = pending
        .iter()
        .map(|(key, row)| (key, sort_key(row, id_field)))
        .collect();
    entries.sort_by(|left, right| left.1.cmp(&right.1));
    entries
        .into_iter()
        .take(DAILY_BATCH_SIZE)
        .map(|(key, _sort)| key.clone())
        .collect()
}

fn sort_key(row: &SpendRow, id_field: &str) -> [String; 5] {
    [
        sort_text(row.get("date")),
        sort_text(row.get(id_field)),
        sort_text(row.get("api_key")),
        sort_text(row.get("model")),
        sort_text(row.get("custom_llm_provider")),
    ]
}

// The source reads each sort value as `value or ""`; these five fields are declared text, so a missing or non-text value sorts as the empty string.
fn sort_text(value: Option<&SpendColumnValue>) -> String {
    match value {
        Some(SpendColumnValue::Text(text)) => text.clone(),
        None
        | Some(
            SpendColumnValue::Null
            | SpendColumnValue::Bool(_)
            | SpendColumnValue::Int(_)
            | SpendColumnValue::Decimal(_)
            | SpendColumnValue::Array(_)
            | SpendColumnValue::Object(_),
        ) => String::new(),
    }
}
