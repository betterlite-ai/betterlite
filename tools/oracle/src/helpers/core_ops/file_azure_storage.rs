use std::time::Duration;

use betterlite_core::api::file_azure_storage::naming::{generated_file_name, naming_strategy};
use betterlite_logging::azure_storage_logging::filename::IdSource;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::OrHelperError as _;
use crate::helpers::{Helper, HelperError, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<FileNameCall>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileNameCall {
    filename: String,
    strategy: String,
    now_seconds: f64,
    uuid: String,
}

struct FixedId(String);

impl IdSource for FixedId {
    fn next_id(&self) -> String {
        self.0.clone()
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let answers = input
            .calls
            .iter()
            .map(file_name_answer)
            .collect::<Result<Vec<Value>, Box<HelperError>>>()?;
        Ok(Value::Array(answers))
    }
}

fn file_name_answer(call: &FileNameCall) -> Result<Value, Box<HelperError>> {
    let now = Duration::try_from_secs_f64(call.now_seconds).or_value_error()?;
    let id = uuid::Uuid::parse_str(&call.uuid)
        .or_value_error()?
        .to_string();
    let name = generated_file_name(
        &call.filename,
        naming_strategy(&call.strategy),
        now,
        &FixedId(id),
    );
    Ok(json!({ "name": name }))
}
