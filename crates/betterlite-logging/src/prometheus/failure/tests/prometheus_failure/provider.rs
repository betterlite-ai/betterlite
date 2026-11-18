use crate::prometheus::failure::provider::{
    ProviderRequestData, extract_api_provider_from_request_data,
};
use crate::prometheus::failure::tests::support::{ProviderFixture, capture_logs, metadata};

#[test]
fn the_litellm_params_provider_wins() {
    let fixture = ProviderFixture::new();
    let params = metadata(&[("custom_llm_provider", "openai")]);
    let standard_logging = metadata(&[("custom_llm_provider", "anthropic")]);
    let request = ProviderRequestData {
        litellm_params: Some(&params),
        standard_logging: Some(&standard_logging),
        model: Some("claude-sonnet-4-5"),
    };
    let provider = extract_api_provider_from_request_data(&request, &fixture.context());
    assert_eq!(provider.as_deref(), Some("openai"));
}

#[test]
fn the_standard_logging_provider_comes_next() {
    let fixture = ProviderFixture::new();
    let params = metadata(&[]);
    let standard_logging = metadata(&[("custom_llm_provider", "anthropic")]);
    let request = ProviderRequestData {
        litellm_params: Some(&params),
        standard_logging: Some(&standard_logging),
        model: Some("gpt-4o"),
    };
    let provider = extract_api_provider_from_request_data(&request, &fixture.context());
    assert_eq!(provider.as_deref(), Some("anthropic"));
}

#[test]
fn the_model_is_inferred_when_no_provider_is_declared() {
    let fixture = ProviderFixture::new();
    let params = metadata(&[]);
    let standard_logging = metadata(&[]);
    let request = ProviderRequestData {
        litellm_params: Some(&params),
        standard_logging: Some(&standard_logging),
        model: Some("gpt-4o"),
    };
    let provider = extract_api_provider_from_request_data(&request, &fixture.context());
    assert_eq!(provider.as_deref(), Some("openai"));
}

#[test]
fn the_litellm_params_model_is_inferred_before_the_request_model() {
    let fixture = ProviderFixture::new();
    let params = metadata(&[("model", "gpt-4o")]);
    let request = ProviderRequestData {
        litellm_params: Some(&params),
        standard_logging: None,
        model: Some("claude-sonnet-4-5"),
    };
    let provider = extract_api_provider_from_request_data(&request, &fixture.context());
    assert_eq!(provider.as_deref(), Some("openai"));
}

#[test]
fn an_empty_litellm_params_model_falls_to_the_request_model() {
    let fixture = ProviderFixture::new();
    let params = metadata(&[("model", "")]);
    let request = ProviderRequestData {
        litellm_params: Some(&params),
        standard_logging: None,
        model: Some("claude-sonnet-4-5"),
    };
    let provider = extract_api_provider_from_request_data(&request, &fixture.context());
    assert_eq!(provider.as_deref(), Some("anthropic"));
}

#[test]
fn an_empty_declared_provider_falls_through() {
    let fixture = ProviderFixture::new();
    let params = metadata(&[("custom_llm_provider", "")]);
    let standard_logging = metadata(&[("custom_llm_provider", "anthropic")]);
    let request = ProviderRequestData {
        litellm_params: Some(&params),
        standard_logging: Some(&standard_logging),
        model: Some("gpt-4o"),
    };
    let provider = extract_api_provider_from_request_data(&request, &fixture.context());
    assert_eq!(provider.as_deref(), Some("anthropic"));
}

#[test]
fn absent_maps_still_reach_the_declared_provider() {
    let fixture = ProviderFixture::new();
    let standard_logging = metadata(&[("custom_llm_provider", "anthropic")]);
    let request = ProviderRequestData {
        litellm_params: None,
        standard_logging: Some(&standard_logging),
        model: None,
    };
    let provider = extract_api_provider_from_request_data(&request, &fixture.context());
    assert_eq!(provider.as_deref(), Some("anthropic"));
}

#[test]
fn a_prefixed_model_names_its_declared_provider() {
    let fixture = ProviderFixture::new();
    let request = ProviderRequestData {
        litellm_params: None,
        standard_logging: None,
        model: Some("openai/gpt-4o"),
    };
    let provider = extract_api_provider_from_request_data(&request, &fixture.context());
    assert_eq!(provider.as_deref(), Some("openai"));
}

#[test]
fn an_unknown_model_names_no_provider() {
    let fixture = ProviderFixture::new();
    let request = ProviderRequestData {
        litellm_params: None,
        standard_logging: None,
        model: Some("brand-new-unknown"),
    };
    let provider = extract_api_provider_from_request_data(&request, &fixture.context());
    assert_eq!(provider, None);
}

#[test]
fn an_unknown_model_leaves_a_diagnostic_debug_record() {
    let fixture = ProviderFixture::new();
    let (logs, guard) = capture_logs();
    let request = ProviderRequestData {
        litellm_params: None,
        standard_logging: None,
        model: Some("brand-new-unknown"),
    };
    let provider = extract_api_provider_from_request_data(&request, &fixture.context());
    drop(guard);
    assert_eq!(provider, None);
    assert!(logs.text().contains("could not infer api_provider"));
}

#[test]
fn no_model_and_no_declared_provider_names_none() {
    let fixture = ProviderFixture::new();
    let request = ProviderRequestData {
        litellm_params: None,
        standard_logging: None,
        model: None,
    };
    let provider = extract_api_provider_from_request_data(&request, &fixture.context());
    assert_eq!(provider, None);
}

#[test]
fn an_empty_model_names_no_provider() {
    let fixture = ProviderFixture::new();
    let request = ProviderRequestData {
        litellm_params: None,
        standard_logging: None,
        model: Some(""),
    };
    let provider = extract_api_provider_from_request_data(&request, &fixture.context());
    assert_eq!(provider, None);
}
