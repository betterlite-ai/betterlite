#![cfg(test)]

use rstest::rstest;

use crate::otel::vocabulary::registry::{
    LiteLLMSpanKind, RegistryError, SPAN_REGISTRY, SpanRole, SpanSpec, validate_registry,
};

const REQUEST: SpanSpec = SpanSpec {
    role: SpanRole::ProxyRequest,
    kind: LiteLLMSpanKind::Server,
    parent: None,
};

#[test]
fn shipped_registry_holds_its_invariants() {
    assert_eq!(validate_registry(&SPAN_REGISTRY), Ok(()));
}

#[test]
fn only_the_proxy_request_starts_a_trace() {
    let roots: Vec<SpanSpec> = SPAN_REGISTRY
        .into_iter()
        .filter(|spec| spec.parent.is_none())
        .collect();
    assert_eq!(roots, vec![REQUEST]);
}

#[rstest]
#[case::duplicate(
    vec![REQUEST, REQUEST],
    RegistryError::DuplicateRole { role: SpanRole::ProxyRequest }
)]
#[case::unknown_parent(
    SPAN_REGISTRY[1..].to_vec(),
    RegistryError::UnknownParent { role: SpanRole::LlmCall, parent: SpanRole::ProxyRequest }
)]
#[case::missing(
    SPAN_REGISTRY[..5].to_vec(),
    RegistryError::MissingRoles { roles: vec![SpanRole::DbCall, SpanRole::Service] }
)]
fn broken_registries_are_refused(#[case] registry: Vec<SpanSpec>, #[case] expected: RegistryError) {
    assert_eq!(validate_registry(&registry), Err(expected));
}

#[rstest]
#[case::role(SpanRole::McpListTools.to_string(), "mcp_list_tools")]
#[case::kind(LiteLLMSpanKind::Consumer.to_string(), "consumer")]
fn roles_and_kinds_keep_litellm_values(#[case] value: String, #[case] expected: &str) {
    assert_eq!(value, expected);
}
