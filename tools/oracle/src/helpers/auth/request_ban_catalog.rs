use betterlite_auth::request_ban_catalog::{RequestBanCatalog, observability};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

#[derive(Debug, serde::Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Catalog,
    Observability,
}

#[derive(serde::Serialize)]
struct Catalog {
    banned: Vec<&'static str>,
    nested_config: [&'static str; 2],
    metadata: [&'static str; 2],
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let output = match input {
            Input::Catalog => {
                let pinned = RequestBanCatalog::pinned();
                serde_json::to_value(Catalog {
                    banned: pinned.banned,
                    nested_config: pinned.nested_config,
                    metadata: pinned.metadata,
                })
            }
            Input::Observability => serde_json::to_value(observability::banned_fields()),
        };
        output.map_err(|error| {
            Box::new(HelperError {
                class: "SerializationError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })
    }
}
