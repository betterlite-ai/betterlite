use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    pub operation: String,
    #[serde(flatten)]
    pub fields: BTreeMap<String, Value>,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let output = if input.operation == "tool_params_omit_only_null" {
            let params: betterlite_openai::api::vector_store_records::VectorStoreToolParams =
                serde_json::from_value(json!({
                    "filters": input.fields.get("filters"),
                    "max_num_results": input.fields.get("max_num_results"),
                    "ranking_options": input.fields.get("ranking_options")
                }))
                .map_err(|error| {
                    Box::new(crate::helpers::HelperError {
                        class: "InvalidInput".to_owned(),
                        status: None,
                        error_type: None,
                        param: None,
                        code: None,
                        message: error.to_string(),
                    })
                })?;
            serde_json::to_value(params.to_dict())
        } else {
            Ok(json!(input.fields))
        };
        output.map_err(|error| {
            Box::new(crate::helpers::HelperError {
                class: "InvalidInput".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })
    }
}
