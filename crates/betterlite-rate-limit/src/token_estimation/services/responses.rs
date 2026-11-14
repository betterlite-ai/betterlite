use betterlite_context::local_ttl_cache::dual::DualLocalCache;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::responses_input_messages::{InputRequest, input_messages};
use betterlite_models::decisions::Json;

use super::{ResponsesInput, ResponsesInputRequest};

pub struct CachedResponsesInput<'cache> {
    pub cache: &'cache DualLocalCache,
    pub limits: ApiLimits,
}

impl ResponsesInput for CachedResponsesInput<'_> {
    fn transform_responses_api_input(&self, request: &ResponsesInputRequest<'_>) -> Vec<Json> {
        let input = InputRequest {
            input: Some(request.input.clone()),
            instructions: request.instructions.map(str::to_owned),
        };
        // a transformation error leaves no countable message; the route rejects the same input later
        let Ok(messages) = input_messages(&input, false, self.limits, self.cache) else {
            return Vec::new();
        };
        messages
            .into_iter()
            .filter_map(|message| serde_json::from_value::<Json>(message).ok())
            .collect()
    }
}
