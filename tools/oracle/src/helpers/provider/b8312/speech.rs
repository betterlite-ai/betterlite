use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_prompts::arguments::json::Object;
use betterlite_provider::regional_host_dispatch::speech::{self, SpeechContext};
use secrecy::SecretString;
use tokio::runtime::Builder;

use super::fixtures::calls::{input_failure, section_settings};
use super::fixtures::speech::SpeechRecorder;
use super::oracle::Case;

// the minimax speech arm over the case's own context, read back as the voice and the params the handler saw
#[rustfmt::skip]
pub(crate) fn view(case: &Case) -> HelperResult {
    let settings = section_settings(case);
    let optional_params = Object::new();
    let litellm_params = Object::new();
    let api_key = case.api_key.as_deref().map(SecretString::from);
    let context = SpeechContext {
        api_base: case.api_base.as_deref(), api_key: api_key.as_ref(),
        extra_headers: None, input: case.input.as_str(),
        litellm_params: &litellm_params, model: &case.model,
        optional_params: &optional_params, ssl_verify: None,
        timeout: None, voice: case.voice.clone(),
        settings: &settings,
    };
    let recorder = SpeechRecorder::default();
    let runtime = Builder::new_current_thread().enable_all().build().or_value_error()?;
    runtime.block_on(speech::minimax(None, &context, &recorder)).map_err(|error| input_failure(error.to_string()))?;
    recorder.take_view()
}
