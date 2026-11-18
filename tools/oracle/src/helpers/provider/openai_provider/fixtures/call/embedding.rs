use betterlite_openai::embedding::EmbeddingRequest;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_provider::openai_dispatch::call::EmbeddingCall;

use super::super::case::{Case, failure};
use super::{encoded, explicit_credentials};

pub(super) fn embedding_call(case: &Case) -> Result<EmbeddingCall, Box<HelperError>> {
    let request = EmbeddingRequest::parse(&encoded(case.optional_params.clone())?)
        .map_err(|error| failure("ValueError", error))?;
    Ok(EmbeddingCall {
        model: case.model.clone(),
        request,
        // the route resolved no format of its own, so the body's encoding_format names it
        encoding_format: None,
        credentials: explicit_credentials(case),
        headers: case.headers.clone(),
        timeout: None,
    })
}
