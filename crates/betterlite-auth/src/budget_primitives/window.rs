use betterlite_db::budget_permission_rows::JsonData;
use serde::Serialize;
use std::collections::BTreeMap;

pub const MAX_WINDOW_BYTES: usize = 0x0010_0000;
pub const MAX_WINDOW_DEPTH: usize = 64;

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct BudgetWindow(BTreeMap<String, JsonData>);

impl AsRef<BTreeMap<String, JsonData>> for BudgetWindow {
    fn as_ref(&self) -> &BTreeMap<String, JsonData> {
        &self.0
    }
}

#[must_use]
pub fn decode_budget_window(bytes: &[u8]) -> BudgetWindow {
    if bytes.len() > MAX_WINDOW_BYTES {
        return BudgetWindow::default();
    }
    let Ok(value) = serde_json::from_slice::<JsonData>(bytes) else {
        return BudgetWindow::default();
    };
    if depth(&value) > MAX_WINDOW_DEPTH {
        return BudgetWindow::default();
    }
    match value {
        JsonData::Object(entries) => BudgetWindow(entries),
        JsonData::Null
        | JsonData::Bool(_)
        | JsonData::Number(_)
        | JsonData::String(_)
        | JsonData::Array(_) => BudgetWindow::default(),
    }
}

fn depth(value: &JsonData) -> usize {
    match value {
        JsonData::Array(items) => items
            .iter()
            .map(depth)
            .max()
            .map_or(1, |inner| 1_usize.saturating_add(inner)),
        JsonData::Object(entries) => entries
            .values()
            .map(depth)
            .max()
            .map_or(1, |inner| 1_usize.saturating_add(inner)),
        JsonData::Null | JsonData::Bool(_) | JsonData::Number(_) | JsonData::String(_) => 0,
    }
}

#[must_use]
pub fn coerce_window(value: &JsonData) -> BudgetWindow {
    match value {
        JsonData::Object(entries) => BudgetWindow(entries.clone()),
        JsonData::String(text) => decode_budget_window(text.as_bytes()),
        JsonData::Null | JsonData::Bool(_) | JsonData::Number(_) | JsonData::Array(_) => {
            BudgetWindow::default()
        }
    }
}

pub trait BudgetWindowModel {
    type Error;

    fn model_dump(&self) -> Result<JsonData, Self::Error>;
}

pub fn coerce_model<M>(model: &M) -> Result<BudgetWindow, M::Error>
where
    M: BudgetWindowModel,
{
    match model.model_dump()? {
        JsonData::Object(entries) => Ok(BudgetWindow(entries)),
        JsonData::Null
        | JsonData::Bool(_)
        | JsonData::Number(_)
        | JsonData::String(_)
        | JsonData::Array(_) => Ok(BudgetWindow::default()),
    }
}
