use betterlite_http::interactions::agents_create_read::logging::{
    AgentLogging, AgentPostCall, AgentPreCall,
};
use betterlite_http::interactions::base::{Logging, PreCallArguments};
use betterlite_models::controls::json_value::JsonValue;
use indexmap::IndexMap;

// LiteLLM's pre_call and post_call write one logging object, so both land in the record B3502 passes
pub(super) struct AgentHttpLogging<'record>(pub(super) &'record mut Logging);

impl AgentLogging for AgentHttpLogging<'_> {
    fn pre_call(&mut self, call: AgentPreCall<'_>) {
        self.0.record_pre_call(
            call.input,
            call.api_key,
            PreCallArguments {
                api_base: call.api_base,
                headers: call.headers,
            },
        );
    }

    fn post_call(&mut self, call: AgentPostCall<'_>) {
        self.0.model_call_details.insert(
            "original_response".to_owned(),
            JsonValue::Text(call.original_response.to_owned()),
        );
        self.0.model_call_details.insert(
            "additional_args".to_owned(),
            JsonValue::Object(IndexMap::new()),
        );
    }
}
