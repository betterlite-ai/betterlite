use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_provider::enterprise_ml_dispatch::chat::SAP_MODEL_VERSION;
use betterlite_provider::enterprise_ml_dispatch::sap::{self, SapRequest};
use betterlite_provider::sap_chat::request::transform_request;

use super::super::case::Case;
use super::super::fixture::Fixture;
use super::super::wire::{Failure, failure};

// _complete_sap: the config merge, then B5845's orchestration body
pub(in crate::b8331) fn sap_chat(case: &Case, fixture: &Fixture) -> Result<(), Failure> {
    let api_key = case.key();
    let request = SapRequest {
        api_key: api_key.as_ref(),
        sap_service_key: None,
        env: fixture.source(),
    };
    let prepared = sap::prepare_chat(request, &fixture.transport, &fixture.sap_clock)
        .map_err(|error| failure(error.status(), &error))?;
    let raw = serde_json::to_value(&case.messages).map_err(|error| failure(None, &error))?;
    let messages: Vec<Json> = serde_json::from_value(raw).map_err(|error| failure(None, &error))?;
    let params = Object::from([(
        String::from("model_version"),
        Json::Text(String::from(SAP_MODEL_VERSION)),
    )]);
    let body = transform_request(&case.model, &messages, &params)
        .map_err(|error| failure(error.status(), &error))?;
    fixture
        .transport
        .keep_json(prepared.url, prepared.headers, &body)
}
