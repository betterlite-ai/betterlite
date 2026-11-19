use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_models::decisions::Json as LogJson;
use betterlite_openai::cloudflare_chat::error::CloudflareChatError;
use betterlite_openai::openai_base_config::Object as ChatObject;
use betterlite_prompts::arguments::json::Object;
use serde_json::json;

use crate::hosted_platform_dispatch::context::CallLogging;
use crate::hosted_platform_dispatch::error::DispatchError;
use crate::hosted_platform_dispatch::handler::HandlerError;

use super::support::{
    Fixture, Recorder, dispatch_chat, headers, logging, redactor, secret, secret_text,
};

fn params(source: &str) -> Object {
    serde_json::from_str(source).expect("the fixture params parse")
}

fn chat_object(params: &Object) -> ChatObject {
    serde_json::from_value(serde_json::to_value(params).expect("the params serialize"))
        .expect("the params convert")
}

#[test]
fn handler_errors_report_their_status_code() {
    assert_eq!(
        HandlerError::Upstream { status_code: 503 }.status_code(),
        Some(503)
    );
    assert_eq!(HandlerError::EmptyReply.status_code(), None);
}

#[tokio::test]
async fn fireworks_maps_extra_body_params_only_for_its_chat_config() {
    let redactor = redactor();
    let (mut state, prepost) = logging(&redactor);
    let mut log = CallLogging {
        state: &mut state,
        env: &prepost,
    };
    let mut fixture = Fixture::new();
    fixture.provider_config = Some(ChatConfigTag::FireworksAi);
    fixture.optional_params = params(
        r#"{"temperature":1,"extra_body":{"chat_template_kwargs":{"reasoning_budget":64}}}"#,
    );
    let handler = Recorder::succeeding();
    dispatch_chat(ChatConfigTag::FireworksAi, &fixture, &handler, &mut log)
        .await
        .expect("the fireworks arm answers");
    let call = handler.first();
    assert_eq!(call.provider_config, Some(ChatConfigTag::FireworksAi));
    let mapped = chat_object(&call.optional_params);
    assert_eq!(mapped.get("temperature"), Some(&json!(1)));
    assert_eq!(mapped.get("reasoning_effort"), Some(&json!(64)));
    assert_eq!(mapped.get("chat_template_kwargs"), None);
    assert_eq!(mapped.get("extra_body"), None);
}

#[tokio::test]
async fn fireworks_keeps_optional_params_for_another_provider_config() {
    let redactor = redactor();
    let (mut state, prepost) = logging(&redactor);
    let mut log = CallLogging {
        state: &mut state,
        env: &prepost,
    };
    let mut fixture = Fixture::new();
    fixture.provider_config = Some(ChatConfigTag::TogetherAiChat);
    fixture.optional_params = params(
        r#"{"temperature":1,"extra_body":{"chat_template_kwargs":{"reasoning_budget":64}}}"#,
    );
    let handler = Recorder::succeeding();
    dispatch_chat(ChatConfigTag::FireworksAi, &fixture, &handler, &mut log)
        .await
        .expect("the fireworks arm answers");
    let call = handler.first();
    assert_eq!(call.optional_params, fixture.optional_params);
    assert_eq!(call.provider_config, Some(ChatConfigTag::TogetherAiChat));
}

#[tokio::test]
async fn fireworks_logs_the_error_text_and_headers_on_failure() {
    let redactor = redactor();
    let (mut state, prepost) = logging(&redactor);
    let mut log = CallLogging {
        state: &mut state,
        env: &prepost,
    };
    let mut fixture = Fixture::new();
    fixture.provider_config = Some(ChatConfigTag::FireworksAi);
    fixture.headers = Some(headers(&[("x-scope", "tenant")]));
    let handler = Recorder::failing(503);
    let outcome = dispatch_chat(ChatConfigTag::FireworksAi, &fixture, &handler, &mut log).await;
    assert!(matches!(
        outcome,
        Err(DispatchError::Handler(HandlerError::Upstream {
            status_code: 503
        }))
    ));
    assert_eq!(
        state.model_call_details.get("original_response"),
        Some(&LogJson::Text(String::from(
            "the provider answered with status 503"
        )))
    );
    let args = serde_json::to_value(
        state
            .model_call_details
            .get("additional_args")
            .expect("the failure logged its additional args"),
    )
    .expect("the additional args serialize");
    assert_eq!(
        args.get("headers").and_then(|given| given.get("x-scope")),
        Some(&json!("tenant"))
    );
}

#[tokio::test]
async fn together_passes_every_context_field_with_the_provider_config() {
    let redactor = redactor();
    let (mut state, prepost) = logging(&redactor);
    let mut log = CallLogging {
        state: &mut state,
        env: &prepost,
    };
    let mut fixture = Fixture::new();
    fixture.provider = String::from("together_ai");
    fixture.provider_config = Some(ChatConfigTag::TogetherAiChat);
    fixture.model = String::from("meta-llama/Llama-3.3-70B-Instruct-Turbo");
    fixture.api_base = Some(String::from("https://together.example.com/v1"));
    fixture.api_key = Some(secret("sk-together"));
    fixture.headers = Some(headers(&[("x-scope", "tenant")]));
    fixture.stream = Some(true);
    fixture.acompletion = true;
    fixture.timeout = Some(Duration::from_secs(9));
    fixture.litellm_params = params(r#"{"user":"oracle"}"#);
    fixture.optional_params = params(r#"{"temperature":1}"#);
    let handler = Recorder::succeeding();
    dispatch_chat(ChatConfigTag::TogetherAiChat, &fixture, &handler, &mut log)
        .await
        .expect("the together arm answers");
    let call = handler.first();
    assert_eq!(call.model, fixture.model);
    assert_eq!(call.stream, Some(true));
    assert!(call.acompletion);
    assert_eq!(call.api_base, fixture.api_base);
    assert_eq!(
        secret_text(call.api_key.as_ref()),
        Some(String::from("sk-together"))
    );
    assert_eq!(call.custom_llm_provider, "together_ai");
    assert_eq!(call.headers, fixture.headers);
    assert_eq!(call.litellm_params, fixture.litellm_params);
    assert_eq!(call.optional_params, fixture.optional_params);
    assert_eq!(call.messages, fixture.messages);
    assert_eq!(call.timeout, fixture.timeout);
    assert_eq!(call.provider_config, Some(ChatConfigTag::TogetherAiChat));
}

#[tokio::test]
async fn together_logs_the_error_text_and_headers_on_failure() {
    let redactor = redactor();
    let (mut state, prepost) = logging(&redactor);
    let mut log = CallLogging {
        state: &mut state,
        env: &prepost,
    };
    let mut fixture = Fixture::new();
    fixture.provider = String::from("together_ai");
    fixture.headers = Some(headers(&[("x-scope", "tenant")]));
    let handler = Recorder::failing(429);
    let outcome = dispatch_chat(ChatConfigTag::TogetherAiChat, &fixture, &handler, &mut log).await;
    assert!(matches!(
        outcome,
        Err(DispatchError::Handler(HandlerError::Upstream {
            status_code: 429
        }))
    ));
    assert_eq!(
        state.model_call_details.get("original_response"),
        Some(&LogJson::Text(String::from(
            "the provider answered with status 429"
        )))
    );
}

#[tokio::test]
async fn groq_resolves_the_base_and_key_chains_in_order() {
    let redactor = redactor();
    let (mut state, prepost) = logging(&redactor);
    let mut log = CallLogging {
        state: &mut state,
        env: &prepost,
    };
    let bases = [
        (
            Some("http://passed.example/v1"),
            Some("http://global.example/v1"),
            Some("http://env.example/v1"),
            "http://passed.example/v1",
        ),
        (
            None,
            Some("http://global.example/v1"),
            Some("http://env.example/v1"),
            "http://global.example/v1",
        ),
        (
            None,
            None,
            Some("http://env.example/v1"),
            "http://env.example/v1",
        ),
        (None, None, None, "https://api.groq.com/openai/v1"),
        (
            Some(""),
            Some("http://global.example/v1"),
            None,
            "http://global.example/v1",
        ),
    ];
    for (passed, global, environment, expected) in bases {
        let mut fixture = Fixture::new();
        fixture.provider = String::from("groq");
        fixture.api_base = passed.map(str::to_owned);
        fixture.settings.api_base = global.map(str::to_owned);
        fixture.settings.environment.groq_api_base = environment.map(str::to_owned);
        let handler = Recorder::succeeding();
        dispatch_chat(ChatConfigTag::GroqChat, &fixture, &handler, &mut log)
            .await
            .expect("the groq arm answers");
        assert_eq!(handler.first().api_base.as_deref(), Some(expected));
    }
    let keys = [
        (
            Some("sk-passed"),
            Some("sk-global"),
            Some("sk-groq"),
            Some("sk-env"),
            Some("sk-passed"),
        ),
        (
            None,
            Some("sk-global"),
            Some("sk-groq"),
            Some("sk-env"),
            Some("sk-global"),
        ),
        (None, None, Some("sk-groq"), Some("sk-env"), Some("sk-groq")),
        (None, None, None, Some("sk-env"), Some("sk-env")),
        (None, None, None, None, None),
        (Some(""), Some("sk-global"), None, None, Some("sk-global")),
    ];
    for (passed, global, groq, environment, expected) in keys {
        let mut fixture = Fixture::new();
        fixture.provider = String::from("groq");
        fixture.api_key = passed.map(secret);
        fixture.settings.api_key = global.map(secret);
        fixture.settings.groq_key = groq.map(secret);
        fixture.settings.environment.groq_api_key = environment.map(secret);
        let handler = Recorder::succeeding();
        dispatch_chat(ChatConfigTag::GroqChat, &fixture, &handler, &mut log)
            .await
            .expect("the groq arm answers");
        assert_eq!(
            secret_text(handler.first().api_key.as_ref()).as_deref(),
            expected
        );
    }
    let mut fixture = Fixture::new();
    fixture.provider = String::from("groq");
    fixture.settings.headers = Some(headers(&[("x-scope", "global")]));
    let handler = Recorder::succeeding();
    dispatch_chat(ChatConfigTag::GroqChat, &fixture, &handler, &mut log)
        .await
        .expect("the groq arm answers");
    assert_eq!(
        handler.first().headers,
        Some(headers(&[("x-scope", "global")]))
    );
    fixture.headers = Some(BTreeMap::new());
    let handler = Recorder::succeeding();
    dispatch_chat(ChatConfigTag::GroqChat, &fixture, &handler, &mut log)
        .await
        .expect("the groq arm answers");
    assert_eq!(
        handler.first().headers,
        Some(headers(&[("x-scope", "global")]))
    );
    fixture.headers = Some(headers(&[("x-scope", "request")]));
    let handler = Recorder::succeeding();
    dispatch_chat(ChatConfigTag::GroqChat, &fixture, &handler, &mut log)
        .await
        .expect("the groq arm answers");
    assert_eq!(
        handler.first().headers,
        Some(headers(&[("x-scope", "request")]))
    );
}

#[tokio::test]
async fn groq_adds_config_keys_only_where_the_request_lacks_them() {
    let redactor = redactor();
    let (mut state, prepost) = logging(&redactor);
    let mut log = CallLogging {
        state: &mut state,
        env: &prepost,
    };
    let mut fixture = Fixture::new();
    fixture.provider = String::from("groq");
    fixture.optional_params = params(r#"{"temperature":1}"#);
    fixture.groq_config = params(r#"{"temperature":2,"top_p":3}"#);
    let handler = Recorder::succeeding();
    dispatch_chat(ChatConfigTag::GroqChat, &fixture, &handler, &mut log)
        .await
        .expect("the groq arm answers");
    assert_eq!(
        handler.first().optional_params,
        params(r#"{"temperature":1,"top_p":3}"#)
    );
}

#[tokio::test]
async fn groq_and_cloudflare_send_no_provider_config() {
    let redactor = redactor();
    let (mut state, prepost) = logging(&redactor);
    let mut log = CallLogging {
        state: &mut state,
        env: &prepost,
    };
    let mut fixture = Fixture::new();
    fixture.provider = String::from("groq");
    fixture.provider_config = Some(ChatConfigTag::GroqChat);
    let handler = Recorder::succeeding();
    dispatch_chat(ChatConfigTag::GroqChat, &fixture, &handler, &mut log)
        .await
        .expect("the groq arm answers");
    assert_eq!(handler.first().provider_config, None);
    let mut fixture = Fixture::new();
    fixture.provider = String::from("cloudflare");
    fixture.provider_config = Some(ChatConfigTag::CloudflareChat);
    fixture.api_key = Some(secret("sk-cloudflare"));
    fixture.api_base = Some(String::from("https://mock.example.com/v1"));
    let handler = Recorder::succeeding();
    dispatch_chat(ChatConfigTag::CloudflareChat, &fixture, &handler, &mut log)
        .await
        .expect("the cloudflare arm answers");
    assert_eq!(handler.first().provider_config, None);
}

#[tokio::test]
async fn cloudflare_resolves_the_key_and_base_chains_in_order() {
    let redactor = redactor();
    let (mut state, prepost) = logging(&redactor);
    let mut log = CallLogging {
        state: &mut state,
        env: &prepost,
    };
    let keys = [
        (
            Some("sk-passed"),
            Some("sk-cloudflare"),
            Some("sk-global"),
            Some("sk-env"),
            Some("sk-passed"),
        ),
        (
            None,
            Some("sk-cloudflare"),
            Some("sk-global"),
            Some("sk-env"),
            Some("sk-cloudflare"),
        ),
        (
            None,
            None,
            Some("sk-global"),
            Some("sk-env"),
            Some("sk-global"),
        ),
        (None, None, None, Some("sk-env"), Some("sk-env")),
        (None, None, None, None, None),
    ];
    for (passed, cloudflare, global, environment, expected) in keys {
        let mut fixture = cloudflare_fixture();
        fixture.api_key = passed.map(secret);
        fixture.settings.cloudflare_api_key = cloudflare.map(secret);
        fixture.settings.api_key = global.map(secret);
        fixture.settings.environment.cloudflare_api_key = environment.map(secret);
        let handler = Recorder::succeeding();
        dispatch_chat(ChatConfigTag::CloudflareChat, &fixture, &handler, &mut log)
            .await
            .expect("the cloudflare arm answers");
        assert_eq!(
            secret_text(handler.first().api_key.as_ref()).as_deref(),
            expected
        );
    }
    let bases = [
        (
            Some("https://passed.example.com/v1"),
            Some("https://global.example.com/v1"),
            Some("https://env.example.com/v1"),
            "https://passed.example.com/v1/chat/completions",
        ),
        (
            None,
            Some("https://global.example.com/v1"),
            Some("https://env.example.com/v1"),
            "https://global.example.com/v1/chat/completions",
        ),
        (
            None,
            None,
            Some("https://env.example.com/v1"),
            "https://env.example.com/v1/chat/completions",
        ),
        (
            None,
            None,
            None,
            "https://api.cloudflare.com/client/v4/accounts/account-1234/ai/v1/chat/completions",
        ),
    ];
    for (passed, global, environment, expected) in bases {
        let mut fixture = cloudflare_fixture();
        fixture.api_base = passed.map(str::to_owned);
        fixture.settings.api_base = global.map(str::to_owned);
        fixture.settings.environment.cloudflare_api_base = environment.map(str::to_owned);
        let handler = Recorder::succeeding();
        dispatch_chat(ChatConfigTag::CloudflareChat, &fixture, &handler, &mut log)
            .await
            .expect("the cloudflare arm answers");
        assert_eq!(handler.first().api_base.as_deref(), Some(expected));
    }
}

fn cloudflare_fixture() -> Fixture {
    let mut fixture = Fixture::new();
    fixture.provider = String::from("cloudflare");
    fixture.settings.environment.cloudflare_account_id = Some(String::from("account-1234"));
    fixture
}

#[tokio::test]
async fn cloudflare_answers_a_missing_account_id() {
    let redactor = redactor();
    let (mut state, prepost) = logging(&redactor);
    let mut log = CallLogging {
        state: &mut state,
        env: &prepost,
    };
    let mut fixture = Fixture::new();
    fixture.provider = String::from("cloudflare");
    fixture.api_key = Some(secret("sk-cloudflare"));
    let handler = Recorder::succeeding();
    let outcome = dispatch_chat(ChatConfigTag::CloudflareChat, &fixture, &handler, &mut log).await;
    assert!(matches!(
        outcome,
        Err(DispatchError::Cloudflare(
            CloudflareChatError::MissingAccountId
        ))
    ));
}

#[tokio::test]
async fn cloudflare_passes_headers_as_they_came_without_a_global_fallback() {
    let redactor = redactor();
    let (mut state, prepost) = logging(&redactor);
    let mut log = CallLogging {
        state: &mut state,
        env: &prepost,
    };
    let mut fixture = cloudflare_fixture();
    fixture.settings.headers = Some(headers(&[("x-scope", "global")]));
    let handler = Recorder::succeeding();
    dispatch_chat(ChatConfigTag::CloudflareChat, &fixture, &handler, &mut log)
        .await
        .expect("the cloudflare arm answers");
    assert_eq!(handler.first().headers, None);
    fixture.headers = Some(headers(&[("x-scope", "request")]));
    let handler = Recorder::succeeding();
    dispatch_chat(ChatConfigTag::CloudflareChat, &fixture, &handler, &mut log)
        .await
        .expect("the cloudflare arm answers");
    assert_eq!(
        handler.first().headers,
        Some(headers(&[("x-scope", "request")]))
    );
}

#[tokio::test]
async fn cloudflare_sends_its_own_provider_name() {
    let redactor = redactor();
    let (mut state, prepost) = logging(&redactor);
    let mut log = CallLogging {
        state: &mut state,
        env: &prepost,
    };
    let mut fixture = cloudflare_fixture();
    fixture.provider = String::from("fireworks_ai");
    let handler = Recorder::succeeding();
    dispatch_chat(ChatConfigTag::CloudflareChat, &fixture, &handler, &mut log)
        .await
        .expect("the cloudflare arm answers");
    assert_eq!(handler.first().custom_llm_provider, "cloudflare");
}
