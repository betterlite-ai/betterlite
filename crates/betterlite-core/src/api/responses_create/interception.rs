use super::bridge::Created;
use super::error::InterceptionError;
use super::preparation::Prepared;
use crate::api::responses_create_http::config::Headers;
use crate::api::responses_provider_selection::ParameterMap;
use async_trait::async_trait;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_openai::response_input::ResponsesInput;
use betterlite_prompts::arguments::json::Object;
pub struct DispatchArgs<'call> {
    pub model: &'call str,
    pub input: &'call ResponsesInput,
    pub include: Option<&'call Json>,
    pub instructions: Option<&'call str>,
    pub max_output_tokens: Option<i64>,
    pub prompt: Option<&'call Json>,
    pub metadata: Option<&'call Json>,
    pub parallel_tool_calls: Option<bool>,
    pub previous_response_id: Option<&'call str>,
    pub reasoning: Option<&'call Json>,
    pub store: Option<bool>,
    pub background: Option<bool>,
    pub stream: Option<bool>,
    pub temperature: Option<f64>,
    pub text: Option<&'call Json>,
    pub tool_choice: Option<&'call Json>,
    pub tools: Option<&'call [JsonValue]>,
    pub top_p: Option<f64>,
    pub truncation: Option<&'call str>,
    pub user: Option<&'call str>,
    pub extra_headers: Option<&'call Headers>,
    pub extra_query: Option<&'call Json>,
    pub extra_body: Option<&'call Object>,
    pub timeout: Option<f64>,
    pub custom_llm_provider: Option<&'call str>,
    pub kwargs: &'call ParameterMap,
    pub is_async: bool,
}
pub struct FileSearchArgs<'call> {
    pub call: DispatchArgs<'call>,
    pub service_tier: Option<&'call str>,
    pub safety_identifier: Option<&'call str>,
    pub text_format: Option<&'call Json>,
    pub allowed_openai_params: Option<&'call [String]>,
    pub use_chat_completions_api: bool,
}
pub enum Interception<Context> {
    Continue(Context),
    Handled(Box<Created<Context>>),
}
pub type InterceptionResult<Context> = Result<Interception<Context>, InterceptionError>;
#[async_trait]
pub trait CreateInterception<Context: Send>: Send + Sync {
    fn mcp_requested(&self, tools: Option<&[JsonValue]>) -> bool;
    async fn mcp(&self, args: &DispatchArgs<'_>, context: Context) -> InterceptionResult<Context>;
    async fn file_search(
        &self,
        args: &FileSearchArgs<'_>,
        context: Context,
    ) -> InterceptionResult<Context>;
}
pub(super) fn dispatch_args<'call>(
    prep: &'call Prepared<'_>,
    reasoning: Option<&'call Json>,
    tools: Option<&'call [JsonValue]>,
    kwargs: &'call ParameterMap,
) -> DispatchArgs<'call> {
    let call = prep.call;
    DispatchArgs {
        model: &prep.model,
        input: &prep.input,
        include: call.include.as_ref(),
        instructions: call.instructions.as_deref(),
        max_output_tokens: call.max_output_tokens,
        prompt: call.prompt.as_ref(),
        metadata: call.metadata.as_ref(),
        parallel_tool_calls: call.parallel_tool_calls,
        previous_response_id: call.previous_response_id.as_deref(),
        reasoning,
        store: call.store,
        background: call.background,
        stream: call.stream,
        temperature: call.temperature,
        text: prep.text.as_ref(),
        tool_choice: call.tool_choice.as_ref(),
        tools,
        top_p: call.top_p,
        truncation: call.truncation.as_deref(),
        user: call.user.as_deref(),
        extra_headers: prep.extra_headers.as_ref(),
        extra_query: call.extra_query.as_ref(),
        extra_body: call.extra_body.as_ref(),
        timeout: call.timeout,
        custom_llm_provider: prep.provider.as_deref(),
        kwargs,
        is_async: prep.is_async,
    }
}
