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
        Err(reason) => serde_json::json!({"error": reason}),
    }
}

fn convert(record: Option<&JsonData>) -> Result<Option<LiteLLMTeamTable>, String> {
    let Some(value) = record else {
        return Ok(None);
    };
    let Ok(bytes) = serde_json::to_vec(value) else {
        return Err("invalid json".to_owned());
    };
    let Ok(parsed) = TeamRecord::parse(&bytes) else {
        return Err("invalid json".to_owned());
    };
    decode_team_record(parsed.as_ref()).map_err(|error| label(&error))
}

fn label(error: &TeamRecordError) -> String {
    match error {
        TeamRecordError::Record {
            source: TeamRowError::Field { field, .. },
        } => format!("Field {field} should be a valid dictionary"),
        TeamRecordError::Field { .. }
        | TeamRecordError::Record {
            source:
                TeamRowError::TooLarge { .. } | TeamRowError::TooDeep | TeamRowError::InvalidUtf8(_),
        } => "invalid json".to_owned(),
        TeamRecordError::Record {
            source:
                TeamRowError::Json(_)
                | TeamRowError::NotAnObject
                | TeamRowError::MissingIdentity
                | TeamRowError::MissingTeamId,
        }
        | TeamRecordError::NotAnObject
        | TeamRecordError::NotAList
        | TeamRecordError::Member { .. } => "invalid model".to_owned(),
    }
}
