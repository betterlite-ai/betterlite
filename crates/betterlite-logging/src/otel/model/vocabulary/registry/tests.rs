#![cfg(test)]

use crate::otel::vocabulary::registry::{
    SPAN_REGISTRY, SpanRole, child_roles, root_roles, validate_registry,
};
use crate::otel::vocabulary::resolvers::span_role_for_service;

#[test]
fn registry_and_service_roles_are_valid() {
    validate_registry().unwrap();
    assert_eq!(root_roles(), vec![SpanRole::PROXY_REQUEST]);
    assert_eq!(SPAN_REGISTRY.len(), 7);
    assert_eq!(child_roles(SpanRole::PROXY_REQUEST).len(), 6);
    assert_eq!(span_role_for_service("self"), None);
    assert_eq!(
        span_role_for_service("redis_queue"),
        Some(SpanRole::DB_CALL)
    );
    assert_eq!(
        span_role_for_service("budget_reset"),
        Some(SpanRole::SERVICE)
    );
}
