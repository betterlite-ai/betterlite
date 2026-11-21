use betterlite_context::non_default_params::Params;

use crate::api::transcription_call::call::TranscriptionCall;

pub(super) fn request_for(model: &str) -> TranscriptionCall {
    TranscriptionCall {
        model: model.to_owned(),
        language: None,
        prompt: None,
        response_format: None,
        timestamp_granularities: None,
        temperature: None,
        user: None,
        timeout: None,
        api_key: None,
        api_base: None,
        api_version: None,
        max_retries: None,
        custom_llm_provider: None,
        base_url: None,
        atranscription: None,
        client: None,
        extra_headers: None,
        shared_session: None,
        azure_ad_token: None,
        params: Params::new(),
    }
}
