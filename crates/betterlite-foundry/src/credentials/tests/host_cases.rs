use secrecy::SecretString;

use crate::credentials::auth_headers::get_azure_ai_auth_headers;
use crate::credentials::host::{
    ApiKeyHeader, api_key_header_for_base, is_azure_openai_v1_host, is_foundry_base,
};

use super::builders::{auth_request, environment, headers};
use super::support::FixtureSources;

const V1_BASE: &str = "https://res.services.ai.azure.com/openai/v1";
const DEPLOYMENT_BASE: &str = "https://res.services.ai.azure.com/openai/deployments/x";

pub(super) fn foundry_host_boundary() {
    inference_boundaries();
    v1_host_headers();
}

fn inference_boundaries() {
    let cases = [
        (V1_BASE, true),
        (DEPLOYMENT_BASE, false),
        ("http://res.services.ai.azure.com", true),
        ("https://res.services.ai.azure.com:8443/openai/v1", true),
        ("https://user@res.services.ai.azure.com/path", true),
        ("https://services.ai.azure.com", false),
        ("https://xservices.ai.azure.com", false),
        ("https://res.services.ai.azure.com.evil.com", false),
        ("https://res.openai.azure.com", false),
        ("/openai/deployments/x", false),
        ("not a url", false),
        (
            "HTTPS://RES.SERVICES.AI.AZURE.COM/OPENAI/DEPLOYMENTS/x",
            true,
        ),
        (
            "https://res.services.ai.azure.com/v1/openai/deployments",
            false,
        ),
    ];
    for (base, expected) in cases {
        assert_eq!(is_foundry_base(base), expected, "{base}");
    }
}

fn v1_host_headers() {
    let hosts = [
        (V1_BASE, true),
        ("https://res.openai.azure.com/openai/deployments/x", true),
        ("https://RES.OPENAI.AZURE.COM", true),
        ("https://services.ai.azure.com", false),
        ("https://openai.azure.com", false),
        ("https://xservices.ai.azure.com", false),
        ("https://res.services.ai.azure.com.evil.com", false),
        ("https://foo.bar", false),
    ];
    for (base, expected) in hosts {
        assert_eq!(is_azure_openai_v1_host(Some(base)), expected, "{base}");
    }
    assert!(!is_azure_openai_v1_host(None));
    let headers_for = [
        (V1_BASE, ApiKeyHeader::ApiKey),
        ("https://res.openai.azure.com", ApiKeyHeader::ApiKey),
        (DEPLOYMENT_BASE, ApiKeyHeader::ApiKey),
        ("https://foo.bar", ApiKeyHeader::Authorization),
    ];
    for (base, expected) in headers_for {
        assert_eq!(api_key_header_for_base(Some(base)), expected, "{base}");
    }
    assert_eq!(api_key_header_for_base(None), ApiKeyHeader::Authorization);
    assert_eq!(ApiKeyHeader::Authorization.wire_name(), "Authorization");
    assert_eq!(ApiKeyHeader::ApiKey.wire_name(), "api-key");
    assert_eq!(ApiKeyHeader::ApiKeyTitleCase.wire_name(), "Api-Key");
    assert_eq!(
        ApiKeyHeader::OcpApimSubscriptionKey.wire_name(),
        "Ocp-Apim-Subscription-Key"
    );
}

fn assert_key_header(header: ApiKeyHeader, name: &str) {
    let environment = environment(&[]);
    let sources = FixtureSources::new();
    let key = SecretString::from("v1-key".to_owned());
    let result = get_azure_ai_auth_headers(&auth_request(
        Some(&key),
        None,
        &environment,
        header,
        &sources,
    ))
    .unwrap();
    assert_eq!(result, headers(&[(name, "v1-key")]), "{name}");
    assert!(sources.calls().is_empty());
}

pub(super) fn openai_v1_api_key_header() {
    let environment = environment(&[]);
    let sources = FixtureSources::new();
    let key = SecretString::from("v1-key".to_owned());
    let header = api_key_header_for_base(Some(V1_BASE));
    assert_eq!(header, ApiKeyHeader::ApiKey);
    let result = get_azure_ai_auth_headers(&auth_request(
        Some(&key),
        None,
        &environment,
        header,
        &sources,
    ))
    .unwrap();
    assert_eq!(result, headers(&[("api-key", "v1-key")]));
    assert!(sources.calls().is_empty());
    let custom = api_key_header_for_base(Some("https://res.services.ai.azure.com/custom"));
    assert_eq!(custom, ApiKeyHeader::Authorization);
    let bearer = get_azure_ai_auth_headers(&auth_request(
        Some(&key),
        None,
        &environment,
        custom,
        &sources,
    ))
    .unwrap();
    assert_eq!(bearer, headers(&[("Authorization", "Bearer v1-key")]));
    assert!(sources.calls().is_empty());
    assert_key_header(ApiKeyHeader::ApiKeyTitleCase, "Api-Key");
    assert_key_header(
        ApiKeyHeader::OcpApimSubscriptionKey,
        "Ocp-Apim-Subscription-Key",
    );
}
