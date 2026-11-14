#![cfg(test)]

use std::sync::Arc;

use serde_json::value::{RawValue, to_raw_value};

use super::error::AgenticHookError;
use super::{
    AgenticCall, AgenticCallback, AgenticGate, AgenticLogging, AgenticLoopCall, AgenticOverrides,
};
use crate::agentic_loop_plan::{
    AgenticLoopPlan, AgenticLoopRequestPatch, FollowupMessage, JsonMap,
};

struct PlainCallback;

impl AgenticCallback for PlainCallback {
    fn callback_name(&self) -> &str {
        "plain"
    }

    fn overrides(&self) -> AgenticOverrides {
        AgenticOverrides::default()
    }
}

struct ForcedCallback;

#[async_trait::async_trait]
impl AgenticCallback for ForcedCallback {
    fn callback_name(&self) -> &str {
        "forced"
    }

    fn overrides(&self) -> AgenticOverrides {
        AgenticOverrides {
            should_run: true,
            run_loop: false,
            build_plan: true,
            post_response: false,
            cleanup: false,
        }
    }

    async fn should_run_agentic_loop(
        &self,
        call: &AgenticCall<'_>,
    ) -> Result<AgenticGate, AgenticHookError> {
        let mut tools = JsonMap::new();
        tools.insert("tool_type".to_owned(), to_raw_value("websearch").unwrap());
        tools.insert("model".to_owned(), to_raw_value(call.model).unwrap());
        tools.insert(
            "provider".to_owned(),
            to_raw_value(call.custom_llm_provider).unwrap(),
        );
        tools.insert("stream".to_owned(), to_raw_value(&call.stream).unwrap());
        tools.insert(
            "response".to_owned(),
            to_raw_value(call.response.get()).unwrap(),
        );
        tools.insert(
            "request_tools".to_owned(),
            to_raw_value(&call.tools).unwrap(),
        );
        tools.insert(
            "message_count".to_owned(),
            to_raw_value(&call.messages.len()).unwrap(),
        );
        Ok(AgenticGate {
            run: call.stream,
            tools,
        })
    }

    async fn build_agentic_loop_plan(
        &self,
        call: &AgenticLoopCall<'_>,
    ) -> Result<AgenticLoopPlan, AgenticHookError> {
        let logging = call.logging;
        Ok(AgenticLoopPlan {
            run_agentic_loop: true,
            request_patch: Some(AgenticLoopRequestPatch {
                model: Some(call.model.to_owned()),
                messages: Some(call.messages.to_vec()),
                tools: None,
                max_tokens: None,
                optional_params: call.optional_params.clone(),
                kwargs: call.kwargs.clone(),
            }),
            response_override: Some(RawValue::from_string(call.response.get().to_owned()).unwrap()),
            terminate: false,
            stop_reason: logging.and_then(|logging| logging.call_id.clone()),
            metadata: call.tools.clone(),
        })
    }
}

struct CallFixture {
    response: Box<RawValue>,
    messages: Vec<FollowupMessage>,
    requested_tools: Vec<FollowupMessage>,
    gate_tools: JsonMap,
    optional_params: JsonMap,
    logging: AgenticLogging,
    kwargs: JsonMap,
}

impl CallFixture {
    fn call(&self) -> AgenticCall<'_> {
        AgenticCall {
            response: &self.response,
            model: "anthropic/claude-sonnet-4-5",
            messages: &self.messages,
            tools: Some(&self.requested_tools),
            stream: true,
            custom_llm_provider: "anthropic",
            kwargs: &self.kwargs,
        }
    }

    fn loop_call(&self) -> AgenticLoopCall<'_> {
        AgenticLoopCall {
            tools: &self.gate_tools,
            model: "anthropic/claude-sonnet-4-5",
            messages: &self.messages,
            response: &self.response,
            optional_params: &self.optional_params,
            logging: Some(&self.logging),
            stream: true,
            kwargs: &self.kwargs,
        }
    }
}

fn call_fixture() -> CallFixture {
    let mut gate_tools = JsonMap::new();
    gate_tools.insert("tool_type".to_owned(), to_raw_value("websearch").unwrap());
    let mut optional_params = JsonMap::new();
    optional_params.insert(
        "temperature".to_owned(),
        RawValue::from_string("0.2".to_owned()).unwrap(),
    );
    let mut agentic_loop_params = JsonMap::new();
    agentic_loop_params.insert("model".to_owned(), to_raw_value("bedrock/claude").unwrap());
    let mut kwargs = JsonMap::new();
    kwargs.insert(
        "max_tokens".to_owned(),
        RawValue::from_string("512".to_owned()).unwrap(),
    );
    CallFixture {
        response: RawValue::from_string("{\"id\":\"msg_1\"}".to_owned()).unwrap(),
        messages: vec![FollowupMessage(
            RawValue::from_string("{\"role\":\"user\"}".to_owned()).unwrap(),
        )],
        requested_tools: vec![FollowupMessage(
            RawValue::from_string("{\"name\":\"web_search\"}".to_owned()).unwrap(),
        )],
        gate_tools,
        optional_params,
        logging: AgenticLogging {
            call_id: Some("call-1".to_owned()),
            agentic_loop_params: Some(agentic_loop_params),
        },
        kwargs,
    }
}

#[tokio::test]
async fn defaults_match_custom_logger() {
    let fixture = call_fixture();
    let callback: Arc<dyn AgenticCallback> = Arc::new(PlainCallback);

    let overrides = callback.overrides();
    assert!(!overrides.should_run);
    assert!(!overrides.run_loop);
    assert!(!overrides.build_plan);
    assert!(!overrides.post_response);
    assert!(!overrides.cleanup);

    let gate = callback
        .should_run_agentic_loop(&fixture.call())
        .await
        .unwrap();
    assert!(!gate.run);
    assert!(gate.tools.is_empty());

    let loop_call = fixture.loop_call();
    let plan = callback.build_agentic_loop_plan(&loop_call).await.unwrap();
    assert!(!plan.run_agentic_loop);
    assert!(plan.request_patch.is_none());

    assert!(
        callback
            .run_agentic_loop(&loop_call)
            .await
            .unwrap()
            .is_none()
    );

    let response = RawValue::from_string("{\"id\":\"msg_1\"}".to_owned()).unwrap();
    let returned = callback
        .post_agentic_loop_response(response.clone(), &plan, &fixture.kwargs)
        .await
        .unwrap();
    assert_eq!(returned.get(), response.get());

    callback
        .agentic_loop_cleanup(&plan, &fixture.kwargs)
        .await
        .unwrap();
}

#[test]
fn overrides_report_exactly_the_two_hooks() {
    let overrides = ForcedCallback.overrides();
    assert!(overrides.should_run);
    assert!(!overrides.run_loop);
    assert!(overrides.build_plan);
    assert!(!overrides.post_response);
    assert!(!overrides.cleanup);
}

#[tokio::test]
async fn overrides_results_reach_the_trait_object() {
    let fixture = call_fixture();
    let callback: Arc<dyn AgenticCallback> = Arc::new(ForcedCallback);

    let gate = callback
        .should_run_agentic_loop(&fixture.call())
        .await
        .unwrap();
    assert!(gate.run);
    assert_eq!(gate.tools.len(), 7);
    assert_eq!(
        gate.tools.get("model").map(|raw| raw.get()),
        Some("\"anthropic/claude-sonnet-4-5\"")
    );
    assert_eq!(
        gate.tools.get("provider").map(|raw| raw.get()),
        Some("\"anthropic\"")
    );
    assert_eq!(gate.tools.get("stream").map(|raw| raw.get()), Some("true"));
    assert_eq!(
        gate.tools.get("message_count").map(|raw| raw.get()),
        Some("1")
    );

    let plan = callback
        .build_agentic_loop_plan(&fixture.loop_call())
        .await
        .unwrap();
    assert!(plan.run_agentic_loop);
    assert_eq!(plan.stop_reason.as_deref(), Some("call-1"));
    assert_eq!(
        plan.response_override.as_ref().map(|raw| raw.get()),
        Some("{\"id\":\"msg_1\"}")
    );
    assert_eq!(
        plan.metadata.get("tool_type").map(|raw| raw.get()),
        Some("\"websearch\"")
    );
    let patch = plan.request_patch.unwrap();
    assert_eq!(patch.model.as_deref(), Some("anthropic/claude-sonnet-4-5"));
    assert_eq!(patch.messages.as_ref().map(Vec::len), Some(1));
    assert_eq!(
        patch
            .optional_params
            .get("temperature")
            .map(|raw| raw.get()),
        Some("0.2")
    );
    assert_eq!(
        patch.kwargs.get("max_tokens").map(|raw| raw.get()),
        Some("512")
    );
}

#[test]
fn object_keeps_registration_order() {
    let plain: Arc<dyn AgenticCallback> = Arc::new(PlainCallback);
    let forced: Arc<dyn AgenticCallback> = Arc::new(ForcedCallback);
    let callbacks = vec![plain, forced];
    let names: Vec<&str> = callbacks
        .iter()
        .map(|callback| callback.callback_name())
        .collect();
    assert_eq!(names, ["plain", "forced"]);
}

#[test]
fn object_error_names_the_callback_and_message() {
    let error = AgenticHookError::Failed {
        callback: "websearch_interception".to_owned(),
        message: "search timed out".to_owned(),
    };
    assert_eq!(
        error.to_string(),
        "callback websearch_interception failed: search timed out"
    );
}
