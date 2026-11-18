use betterlite_oracle_helper_api::helper::HelperResult;

use super::input::Input;
use super::{credentials, http, signed, transform};

pub(crate) fn run(input: &Input) -> HelperResult {
    let value = match input {
        Input::SignedEmbeddingRequest(case) => signed::run(case),
        Input::ProviderJsonResponse(case) => http::provider_json_response(case),
        Input::EmbeddingUsage(case) | Input::AsyncEmbeddingSingleArn(case) => transform::run(case),
        Input::HttpError(case) => http::http_error(case),
        Input::EmbeddingCredentials(case) => credentials::run(case),
    }?;
    Ok(value)
}
