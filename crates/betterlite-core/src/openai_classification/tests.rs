#![cfg(test)]

use rstest::rstest;

use super::{is_openai_compatible_host, is_openai_compatible_url, parse_route};

#[rstest]
#[case::openai(Some("api.openai.com"), true)]
#[case::openai_subdomain(Some("eu.api.openai.com"), true)]
#[case::azure_classic(Some("openai.azure.com"), true)]
#[case::azure_classic_subdomain(Some("resource.openai.azure.com"), true)]
#[case::azure_cognitive(Some("resource.cognitiveservices.azure.com"), true)]
#[case::uppercase_is_not_lowercased(Some("API.OPENAI.COM"), false)]
#[case::lookalike_glued(Some("xapi.openai.com"), false)]
#[case::lookalike_azure_glued(Some("fakeopenai.azure.com"), false)]
#[case::lookalike_suffix(Some("api.openai.com.attacker.example"), false)]
#[case::lookalike_azure_suffix(Some("openai.azure.com.attacker.example"), false)]
#[case::lookalike_cognitive_suffix(Some("cognitiveservices.azure.com.evil"), false)]
#[case::empty(Some(""), false)]
#[case::missing(None, false)]
fn compatible_host(#[case] hostname: Option<&str>, #[case] expected: bool) {
    assert_eq!(is_openai_compatible_host(hostname), expected);
}

#[rstest]
#[case::openai("https://api.openai.com/v1/chat/completions", true)]
#[case::openai_uppercase_host("HTTPS://API.OPENAI.COM/v1/models", true)]
#[case::openai_percent_encoded_host("https://api%2Eopenai.com/v1/models", true)]
#[case::openai_without_path("http://api.openai.com", true)]
#[case::openai_subdomain("https://us.api.openai.com/v1/models", true)]
#[case::openai_userinfo("https://user:secret@api.openai.com/v1/models", true)]
#[case::openai_port("https://api.openai.com:8443/v1/models", true)]
#[case::openai_scheme_relative("//api.openai.com/v1/models", true)]
#[case::openai_leading_space("  //api.openai.com/v1/models", true)]
#[case::openai_leading_control("\u{1}https://api.openai.com/v1/models", true)]
#[case::openai_tab_in_scheme("ht\ttp://api.openai.com/v1/models", true)]
#[case::azure_native_path("https://resource.openai.azure.com/openai/deployments/gpt/chat", true)]
#[case::azure_v1_path("https://resource.openai.azure.com/v1/models", true)]
#[case::cognitive_openai_marker("https://r.cognitiveservices.azure.com/openai/models", true)]
#[case::cognitive_v1_marker("https://r.cognitiveservices.azure.com/v1/models", true)]
#[case::azure_without_marker("https://resource.openai.azure.com/models", false)]
#[case::cognitive_other_service("https://r.cognitiveservices.azure.com/speech/recognition", false)]
#[case::cognitive_versioned("https://r.cognitiveservices.azure.com/v3.1/models", false)]
#[case::marker_case_sensitive("https://r.cognitiveservices.azure.com/OpenAI/models", false)]
#[case::marker_without_slash("https://r.cognitiveservices.azure.com/v1", false)]
#[case::marker_in_query("https://r.cognitiveservices.azure.com/models?api=/v1/x", false)]
#[case::azure_dot_segment("https://resource.openai.azure.com/v1/../speech/recognition", false)]
#[case::lookalike_glued("https://xapi.openai.com/v1/models", false)]
#[case::lookalike_suffix("https://api.openai.com.attacker.example/v1/models", false)]
#[case::lookalike_azure_suffix("https://openai.azure.com.attacker.example/openai/x", false)]
#[case::another_host_with_marker("https://evil.example/openai/models", false)]
#[case::credential_before_host("https://api.openai.com@evil.example/v1/models", false)]
#[case::backslash_before_host("https://api.openai.com\\@evil.example/v1/models", false)]
#[case::host_space("https://api.openai.com /v1/models", false)]
#[case::trailing_dot("https://api.openai.com./v1/models", false)]
#[case::relative("api.openai.com/v1/models", false)]
#[case::relative_marker("api.openai.com/openai/models", false)]
#[case::empty("", false)]
#[case::blank("   ", false)]
#[case::scheme_without_slashes("http:api.openai.com/v1/models", false)]
#[case::scheme_single_slash("http:/api.openai.com/v1/models", false)]
#[case::scheme_triple_slash("http:///api.openai.com/v1/models", false)]
#[case::scheme_four_slashes("http:////api.openai.com/v1/models", false)]
#[case::scheme_relative_triple_slash("///api.openai.com/v1/models", false)]
#[case::scheme_relative_empty("//?api.openai.com", false)]
#[case::opaque_scheme("mailto:api.openai.com", false)]
#[case::host_without_scheme_colon("api.openai.com:8080/v1/models", false)]
fn compatible_url(#[case] route: &str, #[case] expected: bool) {
    assert_eq!(is_openai_compatible_url(route), expected);
}

#[rstest]
#[case::absolute(
    "https://api.openai.com/v1/models",
    Some("api.openai.com"),
    "/v1/models"
)]
#[case::scheme_relative("//api.openai.com/v1/models", Some("api.openai.com"), "/v1/models")]
#[case::relative("api.openai.com/v1/models", None, "")]
#[case::scheme_without_slashes("http:api.openai.com/v1/models", None, "")]
#[case::triple_slash("///api.openai.com/v1/models", None, "")]
fn parsed_route(#[case] route: &str, #[case] hostname: Option<&str>, #[case] path: &str) {
    let parsed = parse_route(route);
    assert_eq!(parsed.hostname.as_deref(), hostname);
    assert_eq!(parsed.path, path);
}
