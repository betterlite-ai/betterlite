use betterlite_models::decisions::Json;

// LiteLLM asks _get_modified_max_tokens only for these call types, and only under litellm.modify_params
pub const MODIFIED_CALL_TYPES: [&str; 3] = ["completion", "acompletion", "anthropic_messages"];

// B197's get_modified_max_tokens; a failure here never fails the call, it leaves the user's value alone
pub trait MaxTokensModifier: Send + Sync {
    fn modified_max_tokens(
        &self,
        model: &str,
        hf_model_name: Option<&str>,
        messages: Option<&Json>,
        user_max_tokens: i64,
    ) -> Option<i64>;
}
