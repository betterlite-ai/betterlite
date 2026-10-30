use crate::arguments::json::{Json, Object};

const MAX_FLATTEN_WORK: usize = 100_000;
const MAX_FLATTEN_OUTPUT_BYTES: usize = 1_000_000;

#[derive(Default)]
pub(super) struct FlattenBudget {
    work: usize,
    output_bytes: usize,
}

impl FlattenBudget {
    pub(super) fn charge_work(&mut self) -> Option<()> {
        self.work = self.work.checked_add(1)?;
        (self.work <= MAX_FLATTEN_WORK).then_some(())
    }

    pub(super) fn charge_output(&mut self, object: &Object) -> Option<()> {
        self.output_bytes = self
            .output_bytes
            .checked_add(estimated_object_bytes(object))?;
        (self.output_bytes <= MAX_FLATTEN_OUTPUT_BYTES).then_some(())
    }
}

fn estimated_object_bytes(object: &Object) -> usize {
    let mut total: usize = 2;
    let mut pending: Vec<&Json> = object.values().collect();
    for key in object.keys() {
        total = total.saturating_add(key.len().saturating_add(4));
    }
    while let Some(value) = pending.pop() {
        let value_bytes = match value {
            Json::Object(fields) => {
                for key in fields.keys() {
                    total = total.saturating_add(key.len().saturating_add(4));
                }
                pending.extend(fields.values());
                2
            }
            Json::List(items) => {
                pending.extend(items);
                items.len().saturating_sub(1).saturating_add(2)
            }
            Json::Text(text) => text.len().saturating_add(2),
            Json::Bool(true) | Json::Null => 4,
            Json::Bool(false) => 5,
            Json::Number(_) => 24,
        };
        total = total.saturating_add(value_bytes);
    }
    total
}
