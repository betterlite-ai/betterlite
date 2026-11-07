#![cfg(test)]

use std::io;
use std::sync::{Arc, Mutex};

use betterlite_config::sections::settings::JsonValue;
use betterlite_openai::openai_base_config::Object;
use tracing::subscriber::{DefaultGuard, set_default};

use super::{
    BridgeCheck, BridgeRequest, ModelInfoError, ReasoningEffort, ResponsesBridgeCapabilities,
    responses_api_bridge_check,
};

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|error| io::Error::other(error.to_string()))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn capture_debug() -> (Captured, DefaultGuard) {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_max_level(tracing::Level::DEBUG)
        .with_ansi(false)
        .finish();
    (captured, set_default(subscriber))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fact {
    ModelInfoFails,
    FoundrySupportsResponses,
    FoundryChatRejectsTools,
    Gpt5,
    Gpt54Plus,
    Gpt5Search,
}

#[derive(Debug, Default)]
struct Fake {
    model_info: Object,
    resolved_openai_api_base: String,
    facts: Vec<Fact>,
}

impl Fake {
    fn with(mut self, fact: Fact) -> Self {
        self.facts.push(fact);
        self
    }

    fn with_info(mut self, model_info: Object) -> Self {
        self.model_info = model_info;
        self
    }

    fn on_base(mut self, api_base: &str) -> Self {
        self.resolved_openai_api_base = api_base.to_owned();
        self
    }

    fn knows(&self, fact: Fact) -> bool {
        self.facts.contains(&fact)
    }
}

impl ResponsesBridgeCapabilities for Fake {
    fn model_info(
        &self,
        model: &str,
        _custom_llm_provider: &str,
    ) -> Result<Object, ModelInfoError> {
        if self.knows(Fact::ModelInfoFails) {
            return Err(ModelInfoError::Unknown {
                model: model.to_owned(),
            });
        }
        Ok(self.model_info.clone())
    }

    fn resolved_openai_api_base(&self, _api_base: Option<&str>) -> String {
        self.resolved_openai_api_base.clone()
    }

    fn foundry_supports_responses(&self, _model: &str, _api_base: Option<&str>) -> bool {
        self.knows(Fact::FoundrySupportsResponses)
    }

    fn foundry_chat_rejects_tools(
        &self,
        _model: &str,
        _reasoning_effort: Option<&ReasoningEffort>,
    ) -> bool {
        self.knows(Fact::FoundryChatRejectsTools)
    }

    fn is_gpt_5_model(&self, _model: &str) -> bool {
        self.knows(Fact::Gpt5)
    }

    fn is_gpt_5_4_plus_model(&self, _model: &str) -> bool {
        self.knows(Fact::Gpt54Plus)
    }

    fn is_gpt_5_search_model(&self, _model: &str) -> bool {
        self.knows(Fact::Gpt5Search)
    }
}

fn text(value: &str) -> JsonValue {
    JsonValue::String(value.to_owned())
}

fn object(pairs: &[(&str, JsonValue)]) -> Object {
    Object::from_iter(
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), value.clone())),
    )
}

fn tool(pairs: &[(&str, JsonValue)]) -> JsonValue {
    JsonValue::Object(object(pairs))
}

fn responses_info() -> Object {
    object(&[("mode", text("responses"))])
}

fn chat_info() -> Object {
    object(&[("mode", text("chat"))])
}

fn check(model_info: Object, model: &str) -> BridgeCheck {
    BridgeCheck {
        model_info,
        model: model.to_owned(),
    }
}

fn function_tool() -> JsonValue {
    tool(&[
        ("type", text("function")),
        ("function", tool(&[("name", text("lookup"))])),
    ])
}

fn bridge_request<'request>(
    model: &'request str,
    custom_llm_provider: &'request str,
) -> BridgeRequest<'request> {
    BridgeRequest {
        model,
        custom_llm_provider,
        web_search_options: None,
        tools: &[],
        reasoning_effort: None,
        reasoning_summary: None,
        api_base: None,
        route_all_openai_to_responses: false,
    }
}

#[test]
fn route_all_openai_returns_only_the_responses_mode() {
    let fake = Fake::default().with_info(chat_info());
    let mut request = bridge_request("responses/gpt-4o", "openai");
    request.route_all_openai_to_responses = true;
    assert_eq!(
        responses_api_bridge_check(&request, &fake),
        check(responses_info(), "gpt-4o")
    );
}

#[test]
fn route_all_without_the_setting_keeps_the_lookup_info() {
    let fake = Fake::default().with_info(chat_info());
    let request = bridge_request("gpt-4o", "openai");
    assert_eq!(
        responses_api_bridge_check(&request, &fake),
        check(chat_info(), "gpt-4o")
    );
}

#[test]
fn route_all_other_providers_ignore_the_setting() {
    let fake = Fake::default().with_info(chat_info());
    let mut request = bridge_request("claude-sonnet", "anthropic");
    request.route_all_openai_to_responses = true;
    assert_eq!(
        responses_api_bridge_check(&request, &fake),
        check(chat_info(), "claude-sonnet")
    );
}

#[test]
fn prefix_sets_responses_mode_when_the_lookup_has_none() {
    let fake = Fake::default();
    let request = bridge_request("responses/gpt-4o", "openai");
    assert_eq!(
        responses_api_bridge_check(&request, &fake),
        check(responses_info(), "gpt-4o")
    );
}

#[test]
fn prefix_is_kept_when_the_lookup_names_a_mode() {
    let fake = Fake::default().with_info(chat_info());
    let request = bridge_request("responses/gpt-4o", "openai");
    assert_eq!(
        responses_api_bridge_check(&request, &fake),
        check(chat_info(), "responses/gpt-4o")
    );
}

#[test]
fn prefix_is_kept_when_the_lookup_already_says_responses() {
    let fake = Fake::default().with_info(responses_info());
    let request = bridge_request("responses/gpt-4o", "openai");
    assert_eq!(
        responses_api_bridge_check(&request, &fake),
        check(responses_info(), "responses/gpt-4o")
    );
}

#[test]
fn prefix_survives_a_failed_lookup() {
    let fake = Fake::default().with(Fact::ModelInfoFails);
    let request = bridge_request("responses/gpt-4o", "openai");
    assert_eq!(
        responses_api_bridge_check(&request, &fake),
        check(responses_info(), "gpt-4o")
    );
}

#[test]
fn prefix_failed_lookup_logs_at_debug() {
    let (captured, _guard) = capture_debug();
    let fake = Fake::default().with(Fact::ModelInfoFails);
    let request = bridge_request("responses/gpt-4o", "openai");
    let _check = responses_api_bridge_check(&request, &fake);
    let logged = captured.text();
    assert!(logged.contains("model info lookup failed"));
    assert!(logged.contains("responses/gpt-4o"));
}

#[test]
fn prefix_strips_every_responses_segment() {
    let fake = Fake::default();
    let request = bridge_request("responses/azure/responses/gpt-4o", "openai");
    assert_eq!(
        responses_api_bridge_check(&request, &fake),
        check(responses_info(), "azure/gpt-4o")
    );
}

#[test]
fn web_search_marks_xai_responses() {
    let fake = Fake::default();
    let options = JsonValue::Bool(true);
    let mut request = bridge_request("responses/grok-4", "xai");
    request.web_search_options = Some(&options);
    assert_eq!(
        responses_api_bridge_check(&request, &fake),
        check(responses_info(), "grok-4")
    );
}

#[test]
fn web_search_on_other_providers_stays_on_chat() {
    let fake = Fake::default();
    let options = JsonValue::Bool(true);
    let mut request = bridge_request("gpt-4o", "openai");
    request.web_search_options = Some(&options);
    assert_eq!(
        responses_api_bridge_check(&request, &fake),
        check(Object::new(), "gpt-4o")
    );
}

#[test]
fn web_search_xai_without_options_stays_on_chat() {
    let fake = Fake::default();
    let request = bridge_request("grok-4", "xai");
    assert_eq!(
        responses_api_bridge_check(&request, &fake),
        check(Object::new(), "grok-4")
    );
}

#[test]
fn gpt5_summary_with_effort_bridges() {
    let fake = Fake::default().with(Fact::Gpt5);
    let effort = ReasoningEffort::Text("medium".to_owned());
    let summary = text("auto");
    let mut request = bridge_request("gpt-5", "openai");
    request.reasoning_effort = Some(&effort);
    request.reasoning_summary = Some(&summary);
    assert_eq!(
        responses_api_bridge_check(&request, &fake),
        check(responses_info(), "gpt-5")
    );
}

#[test]
fn gpt5_summary_without_effort_stays_on_chat() {
    let fake = Fake::default().with(Fact::Gpt5);
    let summary = text("auto");
    let mut request = bridge_request("gpt-5", "openai");
    request.reasoning_summary = Some(&summary);
    assert_eq!(
        responses_api_bridge_check(&request, &fake),
        check(Object::new(), "gpt-5")
    );
}

#[test]
fn gpt5_effort_without_summary_stays_on_chat() {
    let fake = Fake::default().with(Fact::Gpt5);
    let effort = ReasoningEffort::Text("medium".to_owned());
    let mut request = bridge_request("gpt-5", "openai");
    request.reasoning_effort = Some(&effort);
    assert_eq!(
        responses_api_bridge_check(&request, &fake),
        check(Object::new(), "gpt-5")
    );
}

#[test]
fn gpt5_search_models_stay_on_chat() {
    let fake = Fake::default().with(Fact::Gpt5).with(Fact::Gpt5Search);
    let effort = ReasoningEffort::Text("medium".to_owned());
    let summary = text("auto");
    let mut request = bridge_request("gpt-5-search", "openai");
    request.reasoning_effort = Some(&effort);
    request.reasoning_summary = Some(&summary);
    assert_eq!(
        responses_api_bridge_check(&request, &fake),
        check(Object::new(), "gpt-5-search")
    );
}

#[test]
fn gpt5_on_other_providers_stays_on_chat() {
    let fake = Fake::default().with(Fact::Gpt5);
    let effort = ReasoningEffort::Text("medium".to_owned());
    let summary = text("auto");
    let mut request = bridge_request("gpt-5", "anthropic");
    request.reasoning_effort = Some(&effort);
    request.reasoning_summary = Some(&summary);
    assert_eq!(
        responses_api_bridge_check(&request, &fake),
        check(Object::new(), "gpt-5")
    );
}

#[test]
fn gpt5_4_function_tools_with_unset_effort_on_openai_bridge() {
    let fake = Fake::default()
        .with(Fact::Gpt5)
        .with(Fact::Gpt54Plus)
        .on_base("https://api.openai.com/v1");
    let tools = [function_tool()];
    let mut request = bridge_request("gpt-5.4", "openai");
    request.tools = &tools;
    assert_eq!(
        responses_api_bridge_check(&request, &fake),
        check(responses_info(), "gpt-5.4")
    );
}

#[test]
fn gpt5_4_function_tools_with_unset_effort_on_a_custom_base_stay_on_chat() {
    let fake = Fake::default()
        .with(Fact::Gpt5)
        .with(Fact::Gpt54Plus)
        .on_base("https://chat.example.com/v1");
    let tools = [function_tool()];
    let mut request = bridge_request("gpt-5.4", "openai");
    request.tools = &tools;
    assert_eq!(
        responses_api_bridge_check(&request, &fake),
        check(Object::new(), "gpt-5.4")
    );
}

#[test]
fn gpt5_4_function_tools_with_an_explicit_effort_on_a_custom_base_bridge() {
    let fake = Fake::default()
        .with(Fact::Gpt5)
        .with(Fact::Gpt54Plus)
        .on_base("https://chat.example.com/v1");
    let tools = [function_tool()];
    let effort = ReasoningEffort::Text("low".to_owned());
    let mut request = bridge_request("gpt-5.4", "openai");
    request.tools = &tools;
    request.reasoning_effort = Some(&effort);
    assert_eq!(
        responses_api_bridge_check(&request, &fake),
        check(responses_info(), "gpt-5.4")
    );
}

#[test]
fn gpt5_4_function_tools_on_azure_bridge() {
    let fake = Fake::default()
        .with(Fact::Gpt5)
        .with(Fact::Gpt54Plus)
        .on_base("https://chat.example.com/v1");
    let tools = [function_tool()];
    let mut request = bridge_request("gpt-5.4", "azure");
    request.tools = &tools;
    assert_eq!(
        responses_api_bridge_check(&request, &fake),
        check(responses_info(), "gpt-5.4")
    );
}

#[test]
fn gpt5_4_function_tools_on_a_whitespace_base_bridge() {
    let fake = Fake::default()
        .with(Fact::Gpt5)
        .with(Fact::Gpt54Plus)
        .on_base("   ");
    let tools = [function_tool()];
    let mut request = bridge_request("gpt-5.4", "openai");
    request.tools = &tools;
    assert_eq!(
        responses_api_bridge_check(&request, &fake),
        check(responses_info(), "gpt-5.4")
    );
}

#[test]
fn gpt5_custom_tools_stay_on_chat() {
    let fake = Fake::default()
        .with(Fact::Gpt5)
        .with(Fact::Gpt54Plus)
        .on_base("https://api.openai.com/v1");
    let tools = [tool(&[("type", text("custom")), ("name", text("grammar"))])];
    let mut request = bridge_request("gpt-5.4", "openai");
    request.tools = &tools;
    assert_eq!(
        responses_api_bridge_check(&request, &fake),
        check(Object::new(), "gpt-5.4")
    );
}

#[test]
fn gpt5_non_object_tools_are_not_function_tools() {
    let fake = Fake::default()
        .with(Fact::Gpt5)
        .with(Fact::Gpt54Plus)
        .on_base("https://api.openai.com/v1");
    let tools = [text("function")];
    let mut request = bridge_request("gpt-5.4", "openai");
    request.tools = &tools;
    assert_eq!(
        responses_api_bridge_check(&request, &fake),
        check(Object::new(), "gpt-5.4")
    );
}

#[test]
fn gpt5_4_function_tools_with_an_unparsable_base_stay_on_chat() {
    let fake = Fake::default()
        .with(Fact::Gpt5)
        .with(Fact::Gpt54Plus)
        .on_base("http://[::1");
    let tools = [function_tool()];
    let mut request = bridge_request("gpt-5.4", "openai");
    request.tools = &tools;
    assert_eq!(
        responses_api_bridge_check(&request, &fake),
        check(Object::new(), "gpt-5.4")
    );
}

#[test]
fn gpt5_effort_none_keeps_function_tools_on_chat() {
    let fake = Fake::default()
        .with(Fact::Gpt5)
        .with(Fact::Gpt54Plus)
        .on_base("https://api.openai.com/v1");
    let tools = [function_tool()];
    let effort = ReasoningEffort::Text("none".to_owned());
    let mut request = bridge_request("gpt-5.4", "openai");
    request.tools = &tools;
    request.reasoning_effort = Some(&effort);
    assert_eq!(
        responses_api_bridge_check(&request, &fake),
        check(Object::new(), "gpt-5.4")
    );
}

#[test]
fn gpt5_object_effort_none_with_summary_is_active() {
    let fake = Fake::default()
        .with(Fact::Gpt5)
        .with(Fact::Gpt54Plus)
        .on_base("https://api.openai.com/v1");
    let tools = [function_tool()];
    let effort = ReasoningEffort::Object(object(&[
        ("effort", text("none")),
        ("summary", text("auto")),
    ]));
    let mut request = bridge_request("gpt-5.4", "openai");
    request.tools = &tools;
    request.reasoning_effort = Some(&effort);
    assert_eq!(
        responses_api_bridge_check(&request, &fake),
        check(responses_info(), "gpt-5.4")
    );
}

#[test]
fn gpt5_object_effort_none_without_summary_is_inactive() {
    let fake = Fake::default()
        .with(Fact::Gpt5)
        .with(Fact::Gpt54Plus)
        .on_base("https://api.openai.com/v1");
    let tools = [function_tool()];
    let effort = ReasoningEffort::Object(object(&[("effort", text("none"))]));
    let mut request = bridge_request("gpt-5.4", "openai");
    request.tools = &tools;
    request.reasoning_effort = Some(&effort);
    assert_eq!(
        responses_api_bridge_check(&request, &fake),
        check(Object::new(), "gpt-5.4")
    );
}

#[test]
fn gpt5_5_4_predicate_false_keeps_tools_on_chat() {
    let fake = Fake::default()
        .with(Fact::Gpt5)
        .on_base("https://api.openai.com/v1");
    let tools = [function_tool()];
    let mut request = bridge_request("gpt-5.1", "openai");
    request.tools = &tools;
    assert_eq!(
        responses_api_bridge_check(&request, &fake),
        check(Object::new(), "gpt-5.1")
    );
}

#[test]
fn gpt5_function_tools_need_an_object_payload_or_a_name() {
    let fake = Fake::default()
        .with(Fact::Gpt5)
        .with(Fact::Gpt54Plus)
        .on_base("https://api.openai.com/v1");
    let tools = [tool(&[
        ("type", text("function")),
        ("function", text("lookup")),
    ])];
    let mut request = bridge_request("gpt-5.4", "openai");
    request.tools = &tools;
    assert_eq!(
        responses_api_bridge_check(&request, &fake),
        check(Object::new(), "gpt-5.4")
    );
}

#[test]
fn gpt5_a_named_function_tool_counts() {
    let fake = Fake::default()
        .with(Fact::Gpt5)
        .with(Fact::Gpt54Plus)
        .on_base("https://api.openai.com/v1");
    let tools = [tool(&[
        ("type", text("function")),
        ("name", text("lookup")),
    ])];
    let mut request = bridge_request("gpt-5.4", "openai");
    request.tools = &tools;
    assert_eq!(
        responses_api_bridge_check(&request, &fake),
        check(responses_info(), "gpt-5.4")
    );
}

#[test]
fn foundry_rejecting_chat_bridges_function_tools() {
    let fake = Fake::default()
        .with(Fact::Gpt5)
        .with(Fact::FoundrySupportsResponses)
        .with(Fact::FoundryChatRejectsTools);
    let tools = [function_tool()];
    let mut request = bridge_request("gpt-5.6", "azure_ai");
    request.tools = &tools;
    assert_eq!(
        responses_api_bridge_check(&request, &fake),
        check(responses_info(), "gpt-5.6")
    );
}

#[test]
fn foundry_without_native_responses_stays_on_chat() {
    let fake = Fake::default()
        .with(Fact::Gpt5)
        .with(Fact::Gpt54Plus)
        .on_base("https://api.openai.com/v1");
    let tools = [function_tool()];
    let mut request = bridge_request("gpt-5.4", "azure_ai");
    request.tools = &tools;
    assert_eq!(
        responses_api_bridge_check(&request, &fake),
        check(Object::new(), "gpt-5.4")
    );
}

#[test]
fn foundry_uses_its_own_rule_instead_of_the_gpt_5_4_predicate() {
    let fake = Fake::default()
        .with(Fact::Gpt5)
        .with(Fact::Gpt54Plus)
        .with(Fact::FoundrySupportsResponses);
    let tools = [function_tool()];
    let mut request = bridge_request("gpt-5.4", "azure_ai");
    request.tools = &tools;
    assert_eq!(
        responses_api_bridge_check(&request, &fake),
        check(Object::new(), "gpt-5.4")
    );
}

#[test]
fn foundry_summary_with_effort_bridges_without_tools() {
    let fake = Fake::default()
        .with(Fact::Gpt5)
        .with(Fact::FoundrySupportsResponses);
    let effort = ReasoningEffort::Text("medium".to_owned());
    let summary = text("auto");
    let mut request = bridge_request("gpt-5.6", "azure_ai");
    request.reasoning_effort = Some(&effort);
    request.reasoning_summary = Some(&summary);
    assert_eq!(
        responses_api_bridge_check(&request, &fake),
        check(responses_info(), "gpt-5.6")
    );
}

#[test]
fn foundry_rejecting_tools_with_effort_none_stays_on_chat() {
    let fake = Fake::default()
        .with(Fact::Gpt5)
        .with(Fact::FoundrySupportsResponses)
        .with(Fact::FoundryChatRejectsTools);
    let tools = [function_tool()];
    let effort = ReasoningEffort::Text("none".to_owned());
    let mut request = bridge_request("gpt-5.6", "azure_ai");
    request.tools = &tools;
    request.reasoning_effort = Some(&effort);
    assert_eq!(
        responses_api_bridge_check(&request, &fake),
        check(Object::new(), "gpt-5.6")
    );
}
