use betterlite_db::daily_activity_contracts;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::value::{RawValue, to_raw_value};
use std::collections::BTreeMap;
pub(crate) struct Op;
#[derive(Deserialize)]
pub(crate) struct Input {
    steps: BTreeMap<String, Step>,
}
macro_rules! contracts {
    ($($variant:ident: $input:ty => $output:ty),+ $(,)?) => {
        #[derive(Deserialize)]
        #[serde(tag = "kind", content = "value", rename_all = "snake_case")]
        enum Step { $($variant($input),)+ }
        impl crate::helpers::Helper for Op {
            type Input = Input;
            fn run(input: &Self::Input) -> crate::helpers::HelperResult {
                let values: Result<BTreeMap<_, _>, _> = input.steps.iter().map(|(name, step)| {
                    let value = match step {
                        $(Step::$variant(value) => probe::<_, $output>(value),)+
                    }?;
                    Ok((name, value))
                }).collect();
                values.and_then(serde_json::to_value).map_err(|error| Box::new(crate::helpers::HelperError {
                    class: "ValueError".to_owned(), status: None, error_type: None,
                    param: None, code: None, message: error.to_string(),
                }))
            }
        }
    };
}
contracts! {
    Scope: daily_activity_contracts::DailyActivityScopeInput => daily_activity_contracts::DailyActivityScope,
    KeySpend: daily_activity_contracts::KeySpendRow => daily_activity_contracts::KeySpendRow,
    KeyPage: daily_activity_contracts::KeyPage => daily_activity_contracts::KeyPage,
    KeyMetadata: daily_activity_contracts::KeyMetadataRow => daily_activity_contracts::KeyMetadataRow,
    Export: daily_activity_contracts::ExportRow => daily_activity_contracts::ExportRow,
    Rollup: daily_activity_contracts::RollupMetricsRow => daily_activity_contracts::RollupMetricsRow,
    Grouping: daily_activity_contracts::GroupingSetsRow => daily_activity_contracts::GroupingSetsRow,
    Entity: daily_activity_contracts::EntityRollupRow => daily_activity_contracts::EntityRollupRow,
    Aggregated: daily_activity_contracts::AggregatedRows => daily_activity_contracts::AggregatedRows,
    ExportCursor: daily_activity_contracts::ExportCursor => daily_activity_contracts::ExportCursor,
    SqlQuery: daily_activity_contracts::SqlQuery => daily_activity_contracts::SqlQuery,
    DailyRow: daily_activity_contracts::DailyActivityRow => daily_activity_contracts::DailyActivityRow,
    DailyPage: daily_activity_contracts::DailyRowsPage => daily_activity_contracts::DailyRowsPage,
}
#[derive(Serialize)]
struct Probe<Record> {
    value: Option<Record>,
    fields: BTreeMap<String, (bool, bool)>,
}
fn probe<Input, Record>(input: &Input) -> serde_json::Result<Box<RawValue>>
where
    Input: Serialize,
    Record: Serialize + DeserializeOwned,
{
    let encoded = serde_json::to_string(input)?;
    let record = serde_json::from_str::<Record>(&encoded).ok();
    let mut fields = BTreeMap::new();
    let original: BTreeMap<String, Box<RawValue>> = serde_json::from_str(&encoded)?;
    for field in original.keys() {
        let mut changed = original.clone();
        changed.remove(field);
        let missing = serde_json::from_str::<Record>(&serde_json::to_string(&changed)?).is_ok();
        changed.insert(field.clone(), RawValue::from_string("null".to_owned())?);
        let null = serde_json::from_str::<Record>(&serde_json::to_string(&changed)?).is_ok();
        fields.insert(field.clone(), (missing, null));
    }
    to_raw_value(&Probe {
        value: record,
        fields,
    })
}
