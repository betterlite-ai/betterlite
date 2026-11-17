use betterlite_models::decisions::Json;

use super::support::{
    API_KEY, AZURE, BEARER, Harness, Outcome, PROMPT, ProviderScript, Scenario, VERSION, ok_reply,
    reply, scenario,
};
use crate::images::error::ImageError;

pub(super) fn azure_image_body_and_retry() {
    body_table();
    retry_table();
}

fn body_table() {
    body_model_table();
    body_options_table();
}

fn body_model_table() {
    body_absent_model();
    body_base_model();
}

fn body_absent_model() {
    let mut harness = Harness::new(&[]).with_reply(ok_reply());
    let outcome = harness.run(&Scenario {
        model: Some(String::new()),
        ..scenario("dall-e-3")
    });
    // the deployment route drops the model from the wire, so the logged request data carries it
    assert_eq!(outcome.logger.pre[0].data.get("model"), Some(&Json::Null));
    assert!(outcome.calls[0].body_field("model").is_none());
    assert_eq!(outcome.success().hidden_model, None);
}

fn body_base_model() {
    let mut harness = Harness::new(&[]).with_reply(ok_reply());
    let outcome = harness.run(&Scenario {
        base_model: Some("hidden-dall-e-3".to_owned()),
        ..scenario("my-deployment")
    });
    assert_eq!(
        outcome.logger.pre[0].data.get("model"),
        Some(&Json::Text("hidden-dall-e-3".to_owned()))
    );
    assert!(outcome.calls[0].body_field("model").is_none());
    assert_eq!(
        outcome.success().hidden_model.as_deref(),
        Some("hidden-dall-e-3")
    );
    assert_eq!(
        outcome.url(),
        format!(
            "{AZURE}/openai/deployments/my-deployment/images/generations?api-version={VERSION}"
        )
    );

    // an empty base model counts as absent, so the deployment's own name fills the request data
    let mut harness_2 = Harness::new(&[]).with_reply(ok_reply());
    let outcome_2 = harness_2.run(&Scenario {
        base_model: Some(String::new()),
        ..scenario("my-deployment")
    });
    assert_eq!(
        outcome_2.logger.pre[0].data.get("model"),
        Some(&Json::Text("my-deployment".to_owned()))
    );
}

fn body_options_table() {
    let mut harness = Harness::new(&[]).with_reply(ok_reply());
    let outcome = harness.run(&Scenario {
        optional: r#"{"size":"1024x1024","extra_body":{"size":"512x512","foo":"bar"}}"#.to_owned(),
        ..scenario("dall-e-3")
    });
    assert_eq!(
        outcome.calls[0].body_field("size"),
        Some(&Json::Text("512x512".to_owned()))
    );
    assert_eq!(
        outcome.calls[0].body_field("foo"),
        Some(&Json::Text("bar".to_owned()))
    );
    assert!(outcome.calls[0].body_field("extra_body").is_none());
    assert_eq!(
        outcome.calls[0].body_field("prompt"),
        Some(&Json::Text(PROMPT.to_owned()))
    );
}

fn retry_table() {
    accepted_table();
    rejected_table();
}

// every rejected body stops before the transport, whatever status the wrapper gives it
fn rejected_table() {
    for (rejected, status) in [
        (r#"{"max_retries":null}"#, 422),
        (r#"{"max_retries":"3"}"#, 422),
        (r#"{"max_retries":3.5}"#, 422),
        // LiteLLM unpacks extra_body as a mapping, so null fails like any other non-object
        (r#"{"extra_body":"nope"}"#, 500),
        (r#"{"extra_body":null}"#, 500),
    ] {
        let outcome = attempted(rejected);
        assert_eq!(
            outcome.failure().classification(),
            ("AzureOpenAIError", Some(status)),
            "{rejected}"
        );
        assert!(outcome.calls.is_empty(), "{rejected}");
        assert!(outcome.logger.pre.is_empty(), "{rejected}");
        assert!(outcome.logger.post.is_empty(), "{rejected}");
    }
    // LiteLLM re-raises its own image error, so the sync leg must not wrap this one again
    let retried = attempted(r#"{"max_retries":null}"#);
    assert!(matches!(retried.result, Err(ImageError::MaxRetries)));
    assert_eq!(retried.failure().to_string(), "max retries must be an int");
}

fn accepted_table() {
    // any int passes, a negative one and a bool included, and none reaches the body
    for accepted in [
        r#"{"max_retries":3}"#,
        r#"{"max_retries":-1}"#,
        r#"{"max_retries":true}"#,
    ] {
        let outcome = attempted(accepted);
        assert!(outcome.result.is_ok(), "{accepted}");
        assert!(outcome.calls[0].body_field("max_retries").is_none());
    }
}

fn attempted(optional: &str) -> Outcome {
    let mut harness = Harness::new(&[]).with_reply(ok_reply());
    harness.run(&Scenario {
        optional: optional.to_owned(),
        ..scenario("dall-e-3")
    })
}

pub(super) fn azure_image_auth() {
    auth_table();
    provider_table();
    provider_failures();
}

fn auth_table() {
    let mut harness = Harness::new(&[]).with_reply(ok_reply());
    let outcome = harness.run(&scenario("dall-e-3"));
    assert_eq!(outcome.header("api-key"), Some(API_KEY));
    assert_eq!(outcome.header("authorization"), None);
    assert_eq!(
        outcome.pre_headers().get("api-key"),
        Some(&Json::Text("***REDACTED***".to_owned()))
    );
    assert_eq!(outcome.constructed, 1);

    let mut harness_2 = Harness::new(&[]).with_reply(ok_reply());
    let outcome_2 = harness_2.run(&Scenario {
        api_key: None,
        azure_ad_token: Some(BEARER.to_owned()),
        ..scenario("dall-e-3")
    });
    assert_eq!(
        outcome_2.header("authorization"),
        Some(format!("Bearer {BEARER}").as_str())
    );
    assert_eq!(
        outcome_2.pre_headers().get("Authorization"),
        Some(&Json::Text("***REDACTED***".to_owned()))
    );

    let mut harness_3 = Harness::new(&[]).with_reply(ok_reply());
    let outcome_3 = harness_3.run(&Scenario {
        azure_ad_token: Some(BEARER.to_owned()),
        params: r#"{"azure_ad_token":"params-token"}"#.to_owned(),
        ..scenario("dall-e-3")
    });
    assert_eq!(
        outcome_3.header("authorization"),
        Some(format!("Bearer {BEARER}").as_str())
    );
}

fn provider_table() {
    let mut harness = Harness::new(&[]).with_reply(ok_reply());
    let outcome = harness.run(&Scenario {
        api_key: None,
        provider: Some(ProviderScript::Token("provider-token")),
        ..scenario("dall-e-3")
    });
    assert_eq!(
        outcome.header("authorization"),
        Some("Bearer provider-token")
    );

    let mut harness_2 = Harness::new(&[]).with_reply(ok_reply());
    let outcome_2 = harness_2.run(&Scenario {
        headers: vec![("API-KEY".to_owned(), "custom".to_owned())],
        ..scenario("dall-e-3")
    });
    assert_eq!(outcome_2.header("api-key"), Some("custom"));
    assert_eq!(outcome_2.sent_headers().len(), 1);
}

fn provider_failures() {
    let mut harness = Harness::new(&[]).with_reply(ok_reply());
    let outcome = harness.run(&Scenario {
        api_key: None,
        provider: Some(ProviderScript::Failure("RuntimeError", "boom")),
        ..scenario("dall-e-3")
    });
    assert_eq!(
        outcome.failure().classification(),
        ("AzureOpenAIError", Some(500))
    );
    assert_eq!(outcome.failure().to_string(), "boom");
    assert!(outcome.calls.is_empty());

    // a caller that already carries auth never reaches the token provider, so its failure is moot
    let mut harness_2 = Harness::new(&[]).with_reply(ok_reply());
    let outcome_2 = harness_2.run(&Scenario {
        api_key: None,
        headers: vec![("API-KEY".to_owned(), "custom".to_owned())],
        provider: Some(ProviderScript::Failure("RuntimeError", "boom")),
        ..scenario("dall-e-3")
    });
    assert_eq!(outcome_2.success().reply.data.len(), 1);
    assert_eq!(outcome_2.header("api-key"), Some("custom"));
    assert_eq!(outcome_2.sent_headers().len(), 1);

    // an empty provider token counts as no token, so no bearer header goes out
    let mut harness_3 = Harness::new(&[]).with_reply(ok_reply());
    let outcome_3 = harness_3.run(&Scenario {
        api_key: None,
        provider: Some(ProviderScript::Token("")),
        ..scenario("dall-e-3")
    });
    assert_eq!(outcome_3.header("authorization"), None);
    assert_eq!(outcome_3.header("api-key"), None);
}

pub(super) fn azure_image_errors() {
    sync_failure_table();
    async_failure_table();
}

// the reply's HTTP status decides the error, whatever code its body names
const RATE_LIMITED: &str = r#"{"error":{"code":"429","message":"rate limited"}}"#;
const BODY_ERROR: &str = r#"{"error":{"message":"boom","code":400}}"#;

fn failed(status: u16, body: &'static str, delegate_async: bool) -> Outcome {
    let mut harness = Harness::new(&[]).with_reply(reply(status, body));
    harness.run(&Scenario {
        delegate_async,
        ..scenario("dall-e-3")
    })
}

fn sync_failure_table() {
    for (status, body) in [(429, RATE_LIMITED), (503, "Service Unavailable")] {
        let outcome = failed(status, body, false);
        assert_eq!(
            outcome.failure().classification(),
            ("AzureOpenAIError", Some(status))
        );
        assert_eq!(outcome.failure().to_string(), body);
        assert_eq!(outcome.logger.pre.len(), 1);
        assert!(outcome.logger.post.is_empty(), "{status}");
    }

    let outcome = failed(200, BODY_ERROR, false);
    assert_eq!(
        outcome.failure().classification(),
        ("AzureOpenAIError", Some(400))
    );
    assert_eq!(outcome.logger.post.len(), 1);
    assert_eq!(outcome.logger.post[0].input, PROMPT);

    let outcome_2 = failed(200, "not json", false);
    assert_eq!(
        outcome_2.failure().classification(),
        ("AzureOpenAIError", Some(500))
    );
    assert!(outcome_2.logger.post.is_empty());

    let mut harness = Harness::new(&[]).with_reply(ok_reply());
    let outcome_3 = harness.run(&Scenario {
        optional: r#"{"max_retries":null}"#.to_owned(),
        ..scenario("dall-e-3")
    });
    assert_eq!(outcome_3.failure().classification().1, Some(422));
    assert!(outcome_3.calls.is_empty());
    assert!(outcome_3.logger.pre.is_empty());
}

// the async leg logs each failure once as text and re-raises it as it was
fn async_failure_table() {
    for (status, body) in [(429, RATE_LIMITED), (503, "Service Unavailable")] {
        let outcome = failed(status, body, true);
        assert_eq!(
            outcome.failure().classification(),
            ("MaskedHTTPStatusError", Some(status))
        );
        assert_eq!(outcome.logger.post.len(), 1, "{status}");
        assert_eq!(
            outcome.logger.post[0].original_response,
            Json::Text(body.to_owned())
        );
    }

    let outcome = failed(200, "not json", true);
    assert_eq!(outcome.failure().classification(), ("Exception", None));
    assert_eq!(outcome.logger.post.len(), 1);

    let outcome_2 = failed(200, BODY_ERROR, true);
    assert_eq!(
        outcome_2.failure().classification(),
        ("Exception", Some(400))
    );
    assert_eq!(outcome_2.logger.post.len(), 2);
    match &outcome_2.logger.post[1].original_response {
        Json::Text(text) => assert!(text.contains("boom")),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            panic!("the async leg logs the failure as text")
        }
    }
    url_failures();

    let mut harness = Harness::new(&[]).with_reply(ok_reply());
    let outcome_3 = harness.run(&Scenario {
        delegate_async: true,
        ..scenario("dall-e-3")
    });
    assert_eq!(outcome_3.success().reply.size.as_deref(), Some("1024x1024"));
}

// the async leg builds its URL inside the step that logs and re-raises, the sync leg inside the wrapper
fn url_failures() {
    let unusable = Scenario {
        api_base: Some(format!("{AZURE}:port")),
        api_version: Some("preview".to_owned()),
        delegate_async: true,
        ..scenario("dall-e-3")
    };
    let outcome = Harness::new(&[]).with_reply(ok_reply()).run(&unusable);
    assert!(matches!(outcome.result, Err(ImageError::Url(_))));
    assert!(outcome.calls.is_empty());
    assert_eq!(outcome.logger.post.len(), 1);

    let outcome_2 = Harness::new(&[]).with_reply(ok_reply()).run(&Scenario {
        delegate_async: false,
        ..unusable
    });
    assert_eq!(
        outcome_2.failure().classification(),
        ("AzureOpenAIError", Some(500))
    );
    assert!(outcome_2.logger.post.is_empty());
}
