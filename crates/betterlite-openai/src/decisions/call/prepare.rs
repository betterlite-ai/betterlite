use std::sync::{Mutex, PoisonError};

use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::updates::{CallUpdate, update_from_kwargs};
use betterlite_models::decisions::{Extra, Json, ir};
use indexmap::IndexMap;
use secrecy::SecretString;

use crate::decisions::openai::config::Credential;

use super::error::PrepareError;
use super::fields::RequestFormat;
use super::{CredentialInput, Selection, present_secret};
use crate::decisions::openai::request::openai_request_to_ir;
use crate::decisions::transformation::requests::systemone_request_to_ir;
use crate::decisions::transformation::wire::ProviderRequestWire;

pub(super) struct Credentials {
    pub(super) api_base: String,
    pub(super) api_key: Option<SecretString>,
}

pub(super) fn credentials(
    selection: &Selection<'_>,
    input: CredentialInput<'_>,
) -> Result<Credentials, PrepareError> {
    let base_argument = input
        .dynamic_api_base
        .filter(|text| !text.is_empty())
        .or_else(|| input.api_base.filter(|text| !text.is_empty()));
    let resolved_api_base = selection
        .config
        .resolve(Credential::Base, base_argument, selection.environment)
        .ok_or_else(|| PrepareError::MissingApiBase {
            message: selection
                .config
                .ops()
                .missing_api_base_message(selection.provider.as_str()),
            model: selection.model.to_owned(),
            provider: selection.provider.as_str().to_owned(),
        })?;
    let key_argument =
        present_secret(input.dynamic_api_key).or_else(|| present_secret(input.api_key));
    let resolved_api_key =
        selection
            .config
            .resolve(Credential::Key, key_argument, selection.environment);
    if resolved_api_key.is_none() && selection.config.ops().api_key_required() {
        return Err(PrepareError::MissingApiKey {
            model: selection.model.to_owned(),
            provider: selection.provider.as_str().to_owned(),
        });
    }
    Ok(Credentials {
        api_base: resolved_api_base,
        api_key: resolved_api_key.map(SecretString::from),
    })
}

pub(super) fn build_ir(
    selection: &Selection<'_>,
    request: &RequestFormat,
    safety_identifier: Option<&str>,
    dropped: bool,
) -> Result<ir::DecisionRequest, PrepareError> {
    let built = match request {
        RequestFormat::SystemOne(body) => {
            let mut built = systemone_request_to_ir(body);
            built.safety_identifier = safety_identifier.map(str::to_owned);
            built
        }
        RequestFormat::OpenAi(body) => openai_request_to_ir(body),
    };
    if built.safety_identifier.is_none() || selection.config.ops().supports_safety_identifier() {
        return Ok(built);
    }
    if dropped {
        return Ok(ir::DecisionRequest {
            safety_identifier: None,
            ..built
        });
    }
    Err(PrepareError::UnsupportedSafetyIdentifier {
        model: selection.model.to_owned(),
        provider: selection.provider.as_str().to_owned(),
    })
}

pub(super) fn transform_body(
    selection: &Selection<'_>,
    canonical: &str,
    request: &ir::DecisionRequest,
) -> Result<ProviderRequestWire, PrepareError> {
    selection
        .config
        .ops()
        .transform_decisions_request(canonical, request, selection.provider.as_str())
        .map_err(|source| PrepareError::UnsupportedRequest {
            model: selection.model.to_owned(),
            provider: selection.provider.as_str().to_owned(),
            reason: source.to_string(),
        })
}

pub(super) fn log_update(
    selection: &Selection<'_>,
    logging: &Mutex<LoggingState>,
    kwargs: &Extra,
    api_base: &str,
    canonical: &str,
) -> Result<(), PrepareError> {
    let complete_url = selection.config.ops().get_complete_url(api_base, canonical);
    let mut litellm_params = IndexMap::new();
    litellm_params.insert(
        "litellm_call_id".to_owned(),
        kwargs.get("litellm_call_id").cloned().unwrap_or(Json::Null),
    );
    litellm_params.insert("api_base".to_owned(), Json::Text(complete_url));
    let mut additional_params = IndexMap::new();
    additional_params.insert(
        "custom_llm_provider".to_owned(),
        Json::Text(selection.provider.as_str().to_owned()),
    );
    let mut state = logging.lock().unwrap_or_else(PoisonError::into_inner);
    update_from_kwargs(
        &mut state,
        kwargs,
        Some(&litellm_params),
        CallUpdate {
            model: Some(canonical.to_owned()),
            user: None,
            optional_params: IndexMap::new(),
            additional_params,
        },
    )
    .map_err(|source| PrepareError::Logging {
        model: selection.model.to_owned(),
        provider: selection.provider.as_str().to_owned(),
        source,
    })
}
