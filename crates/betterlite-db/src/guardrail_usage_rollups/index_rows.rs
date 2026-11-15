use chrono::{DateTime, Utc};
use indexmap::IndexMap;

use super::payload::{ids_by_name, information_entries, payload_stamp, resolved_id};
use super::{GuardrailIndexRow, GuardrailSpendLog};

impl GuardrailIndexRow {
    #[must_use]
    pub fn new(
        request_id: String,
        guardrail_id: String,
        policy_id: Option<String>,
        instant: DateTime<Utc>,
    ) -> Self {
        let start_time = datetime_text(instant);
        Self {
            request_id,
            guardrail_id,
            policy_id,
            start_time,
            instant,
        }
    }
}

// json.dumps(row, default=str) writes str(datetime): a space separator, six digits of microseconds, the offset
fn datetime_text(instant: DateTime<Utc>) -> String {
    if instant.timestamp_subsec_micros() == 0 {
        return instant.format("%Y-%m-%d %H:%M:%S+00:00").to_string();
    }
    instant.format("%Y-%m-%d %H:%M:%S%.6f+00:00").to_string()
}

// One row per request and guardrail, and a later entry replaces a row only while that row has no policy id
#[must_use]
pub(crate) fn build(payloads: &[GuardrailSpendLog]) -> Vec<GuardrailIndexRow> {
    let mut rows: IndexMap<(String, String), GuardrailIndexRow> = IndexMap::new();
    for payload in payloads {
        let Some(stamp) = payload_stamp(payload) else {
            continue;
        };
        let entries = information_entries(payload);
        let ids = ids_by_name(entries);
        for entry in entries {
            let Some(guardrail_id) = resolved_id(entry, &ids) else {
                continue;
            };
            let policy_id = entry.policy_id.clone();
            let key = (stamp.request_id.clone(), guardrail_id.clone());
            let replace = rows
                .get(&key)
                .is_none_or(|prior| prior.policy_id.is_none() && policy_id.is_some());
            if replace {
                let row = GuardrailIndexRow::new(
                    stamp.request_id.clone(),
                    guardrail_id,
                    policy_id,
                    stamp.instant,
                );
                rows.insert(key, row);
            }
        }
    }
    rows.into_values().collect()
}
