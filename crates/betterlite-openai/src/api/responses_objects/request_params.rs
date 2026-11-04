use betterlite_models::decisions::Json;
use serde_json::Number;

macro_rules! request {
    ($name:ident { required { $($required:ident: $required_type:ty),* $(,)? }
        optional { $($optional:ident: $optional_type:ty),* $(,)? } }) => {
        #[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
        pub struct $name {
            $(pub $required: $required_type,)*
            $(#[serde(default, skip_serializing_if = "Option::is_none", with = "serde_with::rust::double_option")]
            pub $optional: Option<Option<$optional_type>>,)*
        }
    };
}

request! { ResponsesApiStreamOptions {
    required {}
    optional { include_obfuscation: bool }
} }

request! { ResponsesApiRequestParams {
    required { input: Json, model: String }
    optional {
        include: Json,
        instructions: String,
        max_output_tokens: i64,
        metadata: Json,
        parallel_tool_calls: bool,
        previous_response_id: String,
        reasoning: Json,
        store: bool,
        background: bool,
        stream: bool,
        temperature: Number,
        text: Json,
        tool_choice: Json,
        tools: Json,
        top_p: Number,
        truncation: String,
        user: String,
        service_tier: String,
        safety_identifier: String,
        prompt: Json,
        max_tool_calls: i64,
        prompt_cache_key: String,
        prompt_cache_retention: String,
        prompt_cache_options: Json,
        stream_options: ResponsesApiStreamOptions,
        top_logprobs: i64,
        partial_images: i64,
        context_management: Json,
    }
} }
