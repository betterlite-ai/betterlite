use betterlite_harness::session::{parse_output, resolve_model_route};
use serde::Deserialize;

#[derive(Debug, Deserialize, Eq, PartialEq)]
struct Answer {
    value: u32,
}

#[test]
fn parses_final_json_object() {
    let (answer, error) = parse_output::<Answer>(None, "progress\n{\"value\": 7}");
    assert_eq!(answer, Some(Answer { value: 7 }));
    assert!(error.is_none());
}

#[test]
fn strips_proxy_prefix_and_gateway_slash() {
    let (model, gateway) = resolve_model_route(
        Some("litellm_proxy/team"),
        Some(" key "),
        Some(" https://gateway/// "),
        false,
    )
    .expect("route should resolve");
    assert_eq!(model.as_deref(), Some("team"));
    assert_eq!(
        gateway.as_ref().map(|target| target.api_base.as_str()),
        Some("https://gateway")
    );
}
