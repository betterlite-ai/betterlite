use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::team_repository::decoder::{TeamRecord, TeamRecordError, decode_team_record};
use betterlite_db::team_rows::decoder::TeamRowError;
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use serde::Deserialize;
use std::collections::BTreeMap;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    cases: BTreeMap<String, Case>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    records: Vec<Option<JsonData>>,
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let mut output = BTreeMap::new();
        for (name, case) in &input.cases {
            let outcomes = case
                .records
                .iter()
                .map(|record| outcome(record.as_ref()))
                .collect::<Vec<_>>();
            output.insert(name.clone(), outcomes);
        }
        serde_json::to_value(output).map_err(|source| {
            Box::new(crate::helpers::HelperError {
                class: "Output".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: source.to_string(),
            })
        })
    }
}

fn outcome(record: Option<&JsonData>) -> serde_json::Value {
    match convert(record) {
        Ok(model) => serde_json::json!({"model": model}),
        Err(source) => serde_json::json!({"error": label(&source)}),
    }
}

fn convert(record: Option<&JsonData>) -> Result<Option<LiteLLMTeamTable>, TeamRecordError> {
    let Some(value) = record else {
        return Ok(None);
    };
    let bytes = serde_json::to_vec(value).map_err(|source| TeamRecordError::Record {
        source: TeamRowError::Json(source),
    })?;
    let parsed = TeamRecord::parse(&bytes)?;
    decode_team_record(parsed.as_ref())
}

fn label(error: &TeamRecordError) -> String {
    match error {
        TeamRecordError::Field { .. } => "invalid json".to_owned(),
        TeamRecordError::Record { source } => record_label(source),
        TeamRecordError::NotAnObject
        | TeamRecordError::NotAList
        | TeamRecordError::Member { .. } => "invalid model".to_owned(),
    }
}

fn record_label(error: &TeamRowError) -> String {
    match error {
        TeamRowError::Field { field, .. } => {
            format!("Field {field} should be a valid dictionary")
        }
        TeamRowError::TooLarge { .. }
        | TeamRowError::TooDeep
        | TeamRowError::InvalidUtf8(_)
        | TeamRowError::Json(_) => "invalid json".to_owned(),
        TeamRowError::NotAnObject | TeamRowError::MissingIdentity | TeamRowError::MissingTeamId => {
            "invalid model".to_owned()
        }
    }
}
