use betterlite_azure::completion::arguments::SentCall;
use betterlite_azure::completion::handler::streaming::streaming;
use betterlite_azure::completion::request::Retries;
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::HelperError;

use super::super::CallCase;
use super::super::hooks::Recorder;
use super::{Fixed, Secrets, block_on, build_call, error_report, runtime, source, stream_report};

// the direct streaming helpers take the request data the public call already built
pub(super) fn direct_stream(case: &CallCase) -> Result<Json, Box<HelperError>> {
    let source = source(case);
    let secrets = Secrets::of(case);
    let mut params = case.optional_params.clone();
    let call = build_call(case, &secrets, &mut params, &source);
    let mut sent = SentCall {
        model: case.model.clone().unwrap_or_default(),
        data: case.data.clone(),
        prompt: String::new(),
        max_retries: Retries(Json::Null),
        litellm_params: &case.litellm_params,
        supplied: Some(source.handle()),
    };
    let fixed = Fixed::default();
    let mut recorder = Recorder::default();
    let result = block_on(streaming(
        &source,
        &mut sent,
        &call,
        &runtime(&fixed),
        &mut recorder,
    ))?;
    match result {
        Ok(stream) => stream_report(&source, &recorder, stream),
        Err(error) => Ok(error_report(&source, &recorder, &error)),
    }
}
