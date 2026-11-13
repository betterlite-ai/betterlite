use betterlite_models::identity::provider_id::ProviderId;
use serde::Deserialize;

use crate::helpers::HelperError;

mod fixtures;
mod reports;
mod tests;

use fixtures::{Fixtures, VertexLists};
use reports::{chat_call, foundry_call, image_report, json_reports, report_each};

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    #[serde(rename = "openai_family_selection")]
    OpenAiFamilySelection {
        #[serde(default)]
        catalog: Catalog,
        calls: Vec<Call>,
    },
    AzureDetectionSelection {
        calls: Vec<Call>,
    },
    FactoryTable {
        #[serde(default)]
        catalog: Catalog,
        #[serde(default)]
        vertex: VertexLists,
        calls: Vec<Call>,
    },
    FoundrySelection {
        calls: Vec<Call>,
    },
    VertexSelection {
        #[serde(default)]
        vertex: VertexLists,
        calls: Vec<Call>,
    },
    CohereAndBedrockSelection {
        calls: Vec<Call>,
    },
    JsonFallbackPrecedence {
        #[serde(default)]
        providers: Vec<JsonProviderEntry>,
        calls: Vec<Call>,
    },
    ImageVariationConfigSelection {
        calls: Vec<Call>,
    },
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub(crate) struct Catalog {
    chat_models: Vec<String>,
    text_models: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Call {
    provider: ProviderId,
    model: String,
    #[serde(default)]
    base_model: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct JsonProviderEntry {
    provider: ProviderId,
    exists: bool,
    #[serde(default)]
    record: bool,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::OpenAiFamilySelection { catalog, calls } => {
                let vertex = VertexLists::default();
                let fixtures = Fixtures::new(catalog, &vertex, &[]);
                report_each(calls, |table, call| chat_call(&fixtures, call, table))
            }
            Input::AzureDetectionSelection { calls }
            | Input::CohereAndBedrockSelection { calls } => {
                let vertex = VertexLists::default();
                let fixtures = Fixtures::new(&Catalog::default(), &vertex, &[]);
                report_each(calls, |table, call| chat_call(&fixtures, call, table))
            }
            Input::FactoryTable {
                catalog,
                vertex,
                calls,
            } => {
                let fixtures = Fixtures::new(catalog, vertex, &[]);
                report_each(calls, |table, call| chat_call(&fixtures, call, table))
            }
            Input::FoundrySelection { calls } => {
                let vertex = VertexLists::default();
                let fixtures = Fixtures::new(&Catalog::default(), &vertex, &[]);
                report_each(calls, |table, call| foundry_call(&fixtures, call, table))
            }
            Input::VertexSelection { vertex, calls } => {
                let fixtures = Fixtures::new(&Catalog::default(), vertex, &[]);
                report_each(calls, |table, call| chat_call(&fixtures, call, table))
            }
            Input::JsonFallbackPrecedence { providers, calls } => {
                let vertex = VertexLists::default();
                let fixtures = Fixtures::new(&Catalog::default(), &vertex, providers);
                Ok(json_reports(&fixtures, calls))
            }
            Input::ImageVariationConfigSelection { calls } => {
                report_each(calls, |_, call| Ok(image_report(call)))
            }
        }
    }
}

fn invalid(message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}
