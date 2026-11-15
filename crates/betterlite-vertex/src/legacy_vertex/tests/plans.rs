use betterlite_models::decisions::{Extra, Json};
use betterlite_models::provider_model_sets::ProviderModelSet;
use rstest::rstest;
use serde_json::Number;

use crate::gemini_options::defaults::GenerationDefaults;
use crate::legacy_vertex::cache::cache_key;
use crate::legacy_vertex::error::LegacyVertexError;
use crate::legacy_vertex::plan::{LegacyRoute, plan};

use super::support::{
    CountingEncoding, PlanFixture, messages, options, request, route_options, services,
};

fn route_name(route: &LegacyRoute) -> &'static str {
    match route {
        LegacyRoute::Vision => "vision",
        LegacyRoute::Chat { .. } => "chat",
        LegacyRoute::Text { fake_stream, .. } => {
            if *fake_stream {
                "code-text"
            } else {
                "text"
            }
        }
        LegacyRoute::Custom { .. } => "custom",
        LegacyRoute::Private { .. } => "private",
    }
}

fn option_text(route: &Extra, name: &str) -> String {
    let value = route.get(name).expect("the planned option");
    serde_json::to_string(value).expect("the option serializes")
}

#[test]
fn string_only_prompt() {
    let encoder = CountingEncoding;
    let projection = services(&encoder);
    let fixture = PlanFixture::new(&[(ProviderModelSet::VertexText, "text-bison")]);
    let contents = messages(
        r#"[
            {"role": "user", "content": "hello"},
            {"role": "assistant", "content": [{"type": "text", "text": "ignored"}]},
            {"role": "user", "content": "world"},
            {"role": "assistant"}
        ]"#,
    );
    let call = request("text-bison", &contents, options("{}"), false);
    let planned = plan(call, &fixture.deps(&projection)).expect("a plan");
    assert_eq!(planned.prompt, "hello world");
}

#[rstest]
#[case::language_over_chat(
    ProviderModelSet::VertexLanguage,
    ProviderModelSet::VertexChat,
    "vision"
)]
#[case::vision_over_chat(ProviderModelSet::VertexVision, ProviderModelSet::VertexChat, "vision")]
#[case::chat_over_text(ProviderModelSet::VertexChat, ProviderModelSet::VertexText, "chat")]
#[case::chat_over_code_text(ProviderModelSet::VertexChat, ProviderModelSet::VertexCodeText, "chat")]
#[case::text_over_code_text(ProviderModelSet::VertexText, ProviderModelSet::VertexCodeText, "text")]
#[case::text_over_code_chat(ProviderModelSet::VertexText, ProviderModelSet::VertexCodeChat, "text")]
#[case::code_text_over_code_chat(
    ProviderModelSet::VertexCodeText,
    ProviderModelSet::VertexCodeChat,
    "code-text"
)]
#[case::chat_over_code_chat(ProviderModelSet::VertexChat, ProviderModelSet::VertexCodeChat, "chat")]
fn model_group_order(
    #[case] first: ProviderModelSet,
    #[case] second: ProviderModelSet,
    #[case] expected: &str,
) {
    let encoder = CountingEncoding;
    let projection = services(&encoder);
    let fixture = PlanFixture::new(&[(first, "shared-model"), (second, "shared-model")]);
    let contents = messages("[]");
    let call = request("shared-model", &contents, options("{}"), false);
    let planned = plan(call, &fixture.deps(&projection)).expect("a plan");
    assert_eq!(route_name(&planned.route), expected);
}

#[test]
fn defaults_only_missing() {
    let encoder = CountingEncoding;
    let projection = services(&encoder);
    let mut fixture = PlanFixture::new(&[(ProviderModelSet::VertexText, "text-bison")]);
    fixture.defaults = GenerationDefaults {
        temperature: Some(0.25_f64),
        max_output_tokens: Some(64),
        top_p: Some(0.5_f64),
        top_k: None,
        response_mime_type: None,
        candidate_count: None,
        stop_sequences: None,
        frequency_penalty: None,
        presence_penalty: None,
        seed: Some(7),
    };
    let contents = messages("[]");
    let call = request(
        "text-bison",
        &contents,
        options(r#"{"temperature": 0.9, "top_p": null, "max_output_tokens": 5}"#),
        false,
    );
    let planned = plan(call, &fixture.deps(&projection)).expect("a plan");
    let route = route_options(&planned.route).expect("the text options");
    assert_eq!(option_text(route, "temperature"), "0.9");
    assert_eq!(option_text(route, "top_p"), "0.5");
    assert_eq!(option_text(route, "max_output_tokens"), "5");
    assert_eq!(option_text(route, "seed"), "7");
}

#[rstest]
#[case::missing("{}", "ok")]
#[case::null(r#"{"safety_settings": null}"#, "ok")]
#[case::empty(r#"{"safety_settings": []}"#, "ok")]
#[case::mapping(r#"{"safety_settings": [{"category": "x"}]}"#, "ok")]
#[case::not_list(r#"{"safety_settings": {}}"#, "list")]
#[case::item(r#"{"safety_settings": [1]}"#, "item")]
#[case::second(r#"{"safety_settings": [{"category": "x"}, 2]}"#, "item")]
fn safety_shape(#[case] source: &str, #[case] expected: &str) {
    let encoder = CountingEncoding;
    let projection = services(&encoder);
    let fixture = PlanFixture::new(&[(ProviderModelSet::VertexChat, "chat-bison")]);
    let contents = messages("[]");
    let call = request("chat-bison", &contents, options(source), false);
    match plan(call, &fixture.deps(&projection)) {
        Ok(planned) => {
            assert_eq!(expected, "ok");
            let route = route_options(&planned.route).expect("the chat options");
            assert!(route.get("safety_settings").is_none());
        }
        Err(error @ LegacyVertexError::SafetyNotList) => {
            assert_eq!(expected, "list");
            assert_eq!(error.status(), 400);
        }
        Err(error @ LegacyVertexError::SafetyItemNotMapping) => {
            assert_eq!(expected, "item");
            assert_eq!(error.status(), 400);
        }
        Err(other) => panic!("unexpected plan error: {other}"),
    }
}

#[test]
fn private_model_id() {
    let encoder = CountingEncoding;
    let projection = services(&encoder);
    let fixture = PlanFixture::new(&[]);
    let contents = messages(r#"[{"role": "user", "content": "hello"}]"#);
    let call = request(
        "private",
        &contents,
        options(r#"{"model_id": "123456", "temperature": 0.2}"#),
        false,
    );
    let planned = plan(call, &fixture.deps(&projection)).expect("a plan");
    assert_eq!(planned.model, "123456");
    assert_eq!(planned.prompt, "hello");
    let LegacyRoute::Private { instances, options } = &planned.route else {
        panic!("the private route");
    };
    assert_eq!(options.get("model_id"), None);
    assert_eq!(instances.len(), 1);
    let Json::Object(instance) = &instances[0] else {
        panic!("an instance object");
    };
    assert_eq!(
        instance.get("prompt"),
        Some(&Json::Text(String::from("hello")))
    );
    assert_eq!(
        instance.get("temperature"),
        Some(&Json::Number(
            Number::from_f64(0.2).expect("a finite float")
        ))
    );
    assert_eq!(instance.get("model_id"), None);
}

#[rstest]
#[case::absent("{}")]
#[case::null(r#"{"model_id": null}"#)]
fn private_requires_model_id(#[case] source: &str) {
    let encoder = CountingEncoding;
    let projection = services(&encoder);
    let fixture = PlanFixture::new(&[]);
    let contents = messages("[]");
    let call = request("private", &contents, options(source), false);
    let Err(error) = plan(call, &fixture.deps(&projection)) else {
        panic!("the private route fails");
    };
    assert!(matches!(error, LegacyVertexError::MissingModelId));
    assert_eq!(error.status(), 500);
}

#[test]
fn group_precedes_private_literal() {
    let encoder = CountingEncoding;
    let projection = services(&encoder);
    let fixture = PlanFixture::new(&[(ProviderModelSet::VertexChat, "private")]);
    let contents = messages("[]");
    let call = request(
        "private",
        &contents,
        options(r#"{"model_id": "123"}"#),
        false,
    );
    let planned = plan(call, &fixture.deps(&projection)).expect("a plan");
    assert_eq!(planned.model, "private");
    let LegacyRoute::Chat { options } = &planned.route else {
        panic!("the chat route");
    };
    assert!(options.contains_key("model_id"));
}

#[rstest]
#[case::no_location(Some("proj"), None)]
#[case::no_project(None, Some("us-central1"))]
#[case::no_context(None, None)]
fn custom_requires_location(#[case] project: Option<&str>, #[case] location: Option<&str>) {
    let encoder = CountingEncoding;
    let projection = services(&encoder);
    let mut fixture = PlanFixture::new(&[]);
    fixture.project = project.map(str::to_owned);
    fixture.location = location.map(str::to_owned);
    let contents = messages("[]");
    let call = request("custom-model", &contents, options("{}"), false);
    let Err(error) = plan(call, &fixture.deps(&projection)) else {
        panic!("the custom route fails");
    };
    assert!(matches!(error, LegacyVertexError::CustomEndpointContext));
    assert_eq!(error.status(), 500);
}

#[test]
fn custom_instance_shape() {
    let encoder = CountingEncoding;
    let projection = services(&encoder);
    let fixture = PlanFixture::new(&[]);
    let contents = messages(r#"[{"role": "user", "content": "hello"}]"#);
    let call = request(
        "custom-model",
        &contents,
        options(
            r#"{"temperature": 0.3, "model_id": "999", "safety_settings": [{"category": "x"}]}"#,
        ),
        false,
    );
    let planned = plan(call, &fixture.deps(&projection)).expect("a plan");
    assert_eq!(planned.model, "custom-model");
    let LegacyRoute::Custom { instances, options } = &planned.route else {
        panic!("the custom route");
    };
    assert_eq!(options.get("safety_settings"), None);
    assert_eq!(option_text(options, "temperature"), "0.3");
    let Json::Object(instance) = &instances[0] else {
        panic!("an instance object");
    };
    let model_id = Json::Text(String::from("999"));
    assert_eq!(instance.get("model_id"), Some(&model_id));
    assert_eq!(options.get("model_id"), Some(&model_id));
    assert_eq!(
        instance.get("prompt"),
        Some(&Json::Text(String::from("hello")))
    );
    assert_eq!(instance.get("safety_settings"), None);
}

#[rstest]
#[case::code_text(ProviderModelSet::VertexCodeText, "code-bison", true)]
#[case::text(ProviderModelSet::VertexText, "text-bison", false)]
fn code_fake_stream(#[case] group: ProviderModelSet, #[case] model: &str, #[case] expected: bool) {
    let encoder = CountingEncoding;
    let projection = services(&encoder);
    let fixture = PlanFixture::new(&[(group, model)]);
    let contents = messages("[]");
    let call = request(model, &contents, options("{}"), false);
    let planned = plan(call, &fixture.deps(&projection)).expect("a plan");
    let LegacyRoute::Text { fake_stream, .. } = &planned.route else {
        panic!("the text route");
    };
    assert_eq!(*fake_stream, expected);
}

#[test]
fn cache_key_spelling() {
    assert_eq!(
        cache_key("text-bison", Some("proj"), Some("us-central1")),
        "text-bison-proj-us-central1"
    );
    assert_eq!(cache_key("text-bison", None, None), "text-bison-None-None");
    assert_eq!(cache_key("text-bison", Some(""), Some("")), "text-bison--");
    assert_eq!(cache_key("a-b", Some("c"), Some("d")), "a-b-c-d");
}
