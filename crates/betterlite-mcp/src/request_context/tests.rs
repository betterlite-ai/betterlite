#![cfg(test)]

use http::Extensions;

use super::McpContext;

#[test]
fn a_caller_built_context_starts_from_the_context_var_defaults() {
    let context = McpContext::default();
    assert_eq!(context.active_toolset_id, None);
    assert_eq!(context.initialize_instructions, None);
    assert_eq!(context.gateway_server_name, None);
    assert!(!context.proxy_mode);
    assert!(context.request_scope.get::<String>().is_none());
}

#[test]
fn the_caller_builds_every_field_the_python_stores() {
    let mut request_scope = Extensions::new();
    request_scope.insert("inbound-request".to_owned());
    let context = McpContext {
        request_scope,
        active_toolset_id: Some("toolset-1".to_owned()),
        initialize_instructions: Some("use the tools".to_owned()),
        gateway_server_name: Some("github".to_owned()),
        proxy_mode: true,
    };
    assert_eq!(
        context.request_scope.get::<String>().map(String::as_str),
        Some("inbound-request")
    );
    assert_eq!(context.active_toolset_id.as_deref(), Some("toolset-1"));
    assert_eq!(
        context.initialize_instructions.as_deref(),
        Some("use the tools")
    );
    assert_eq!(context.gateway_server_name.as_deref(), Some("github"));
    assert!(context.proxy_mode);
}

#[test]
fn two_requests_never_read_each_others_context() {
    let first = McpContext {
        request_scope: Extensions::new(),
        active_toolset_id: Some("toolset-1".to_owned()),
        initialize_instructions: None,
        gateway_server_name: None,
        proxy_mode: true,
    };
    let second = McpContext {
        request_scope: Extensions::new(),
        active_toolset_id: None,
        initialize_instructions: Some("other".to_owned()),
        gateway_server_name: Some("other-server".to_owned()),
        proxy_mode: false,
    };
    assert_eq!(first.active_toolset_id.as_deref(), Some("toolset-1"));
    assert_eq!(second.active_toolset_id, None);
    assert!(first.proxy_mode);
    assert!(!second.proxy_mode);
    assert_eq!(first.initialize_instructions, None);
    assert_eq!(second.gateway_server_name.as_deref(), Some("other-server"));
}
