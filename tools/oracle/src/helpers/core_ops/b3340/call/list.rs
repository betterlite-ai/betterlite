use betterlite_core::api::video_list_call::{ListRequest, VideoListCall, avideo_list, video_list};
use betterlite_core::api::video_remix_call::shared::VideoCallDeps;
use betterlite_oracle_helper_api::error::HelperError;
use indexmap::IndexMap;
use secrecy::SecretString;

use super::super::harness::{Bench, base_env};
use super::super::{Input, failed};
use super::{Outcome, call_timeout, current_runtime, extra_headers, mock, outcome, params};

pub(super) fn run(
    input: &Input,
    bench: &mut Bench,
    is_async: bool,
) -> Result<Outcome, Box<HelperError>> {
    let call = list_call(input)?;
    let pre_post = base_env(&bench.env);
    let deps = VideoCallDeps {
        configs: &bench.configs,
        logging: &mut bench.state,
        env: &pre_post,
        redactor: bench.env.redactor(),
        request_timeout_seconds: input.request_timeout_seconds.unwrap_or(6000.0),
    };
    let request = ListRequest {
        transport: &bench.transport,
        deps,
        call: &call,
        client: None,
    };
    let result = if is_async {
        let resolution = bench.resolver.context();
        current_runtime()?.block_on(avideo_list(request, &resolution))
    } else {
        video_list(request)
    };
    Ok(outcome(result))
}

fn list_call(input: &Input) -> Result<VideoListCall, Box<HelperError>> {
    Ok(VideoListCall {
        after: input.after.clone(),
        limit: input.limit,
        order: input.order.clone(),
        custom_llm_provider: input.custom_llm_provider.clone(),
        api_key: input.api_key.as_deref().map(SecretString::from),
        api_base: input.api_base.clone(),
        api_version: None,
        extra_headers: extra_headers(input),
        extra_query: params(input.extra_query.as_ref())?,
        extra_body: params(input.extra_body.as_ref())?,
        mock_response: mock(input.mock_response.as_ref())?,
        litellm_metadata: None,
        litellm_call_id: Some(input.litellm_call_id.clone()),
        user: input.user.clone(),
        timeout: call_timeout(input),
        extra: IndexMap::new(),
    })
}
