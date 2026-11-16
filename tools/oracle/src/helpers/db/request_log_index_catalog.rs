use betterlite_db::request_log_index_catalog::definitions::RequestLogIndex;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

#[derive(Debug, serde::Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Columns {
        table: String,
        name: String,
        definition: String,
        partition: String,
    },
    PartitionIndexName {
        table: String,
        name: String,
        definition: String,
        partition: String,
    },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Columns {
                table,
                name,
                definition,
                partition: _partition,
            } => {
                let index = RequestLogIndex {
                    table: table.as_str(),
                    name: name.as_str(),
                    definition: definition.as_str(),
                };
                Ok(serde_json::json!(index.columns()))
            }
            Input::PartitionIndexName {
                table,
                name,
                definition,
                partition,
            } => {
                let index = RequestLogIndex {
                    table: table.as_str(),
                    name: name.as_str(),
                    definition: definition.as_str(),
                };
                Ok(serde_json::Value::String(
                    index.partition_index_name(partition),
                ))
            }
        }
    }
}
