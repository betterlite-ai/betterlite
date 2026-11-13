#![cfg(test)]

use betterlite_mcp::credential_contracts::auth_spec::AuthResolution;
use betterlite_mcp::request_context::McpContext;
use rstest::rstest;

use super::{
    AUTH_RESOLUTION_HEADER, AUTH_RESOLUTIONS_HEADER, AUTH_RESOLUTIONS_TRUNCATED,
    MCPAuthDiagnostics, record_auth_resolution,
};

fn header<'headers>(headers: &'headers [(String, String)], name: &str) -> Option<&'headers str> {
    headers
        .iter()
        .find(|(existing, _value)| existing == name)
        .map(|(_name, value)| value.as_str())
}

#[test]
fn an_empty_diagnostic_set_reports_unresolved() {
    let diagnostics = MCPAuthDiagnostics::default();
    assert_eq!(diagnostics.resolution(), AuthResolution::Unresolved);
    assert_eq!(
        diagnostics.headers(),
        vec![(
            String::from(AUTH_RESOLUTION_HEADER),
            String::from("unresolved")
        )]
    );
}

#[test]
fn one_server_reports_its_own_resolution() {
    let mut diagnostics = MCPAuthDiagnostics::default();
    diagnostics.record("s1", AuthResolution::StoredUserToken);
    assert_eq!(diagnostics.resolution(), AuthResolution::StoredUserToken);
    assert_eq!(
        header(&diagnostics.headers(), AUTH_RESOLUTION_HEADER),
        Some("stored-user-token")
    );
}

#[test]
fn several_servers_report_multiple_with_a_json_map() {
    let mut diagnostics = MCPAuthDiagnostics::default();
    diagnostics.record("s1", AuthResolution::StaticToken);
    diagnostics.record("s2", AuthResolution::NoAuth);
    assert_eq!(diagnostics.resolution(), AuthResolution::Multiple);
    let headers = diagnostics.headers();
    assert_eq!(header(&headers, AUTH_RESOLUTION_HEADER), Some("multiple"));
    assert_eq!(
        header(&headers, AUTH_RESOLUTIONS_HEADER),
        Some("{\"s1\":\"static-token\",\"s2\":\"no-auth\"}")
    );
    assert_eq!(header(&headers, AUTH_RESOLUTIONS_TRUNCATED), None);
}

#[test]
fn recording_a_server_again_moves_it_to_the_end() {
    let mut diagnostics = MCPAuthDiagnostics::default();
    diagnostics.record("s1", AuthResolution::StaticToken);
    diagnostics.record("s2", AuthResolution::NoAuth);
    diagnostics.record("s1", AuthResolution::StoredUserToken);
    let headers = diagnostics.headers();
    assert_eq!(
        header(&headers, AUTH_RESOLUTIONS_HEADER),
        Some("{\"s2\":\"no-auth\",\"s1\":\"stored-user-token\"}")
    );
}

#[test]
fn recording_the_same_server_twice_leaves_one_outcome() {
    let mut diagnostics = MCPAuthDiagnostics::default();
    diagnostics.record("s1", AuthResolution::StaticToken);
    diagnostics.record("s1", AuthResolution::NoAuth);
    assert_eq!(diagnostics.resolution(), AuthResolution::NoAuth);
}

#[rstest]
#[case::at_the_limit(32, false)]
#[case::past_the_limit(33, true)]
fn the_json_map_reports_at_most_thirty_two_servers(
    #[case] servers: usize,
    #[case] truncated: bool,
) {
    let mut diagnostics = MCPAuthDiagnostics::default();
    for index in 0..servers {
        diagnostics.record(&format!("server-{index:02}"), AuthResolution::StaticToken);
    }
    let headers = diagnostics.headers();
    let map = header(&headers, AUTH_RESOLUTIONS_HEADER).expect("resolutions");
    assert_eq!(
        header(&headers, AUTH_RESOLUTIONS_TRUNCATED),
        truncated.then_some("true")
    );
    let reported = map.matches(':').count();
    assert_eq!(reported, servers.min(32));
    assert!(!map.contains("server-32"));
}

#[test]
fn the_context_scope_carries_the_diagnostics() {
    let mut context = McpContext::default();
    context.request_scope.insert(MCPAuthDiagnostics::default());
    record_auth_resolution(&mut context, "s1", AuthResolution::PerRequestHeader);
    let diagnostics = context
        .request_scope
        .get::<MCPAuthDiagnostics>()
        .expect("diagnostics in the request scope");
    assert_eq!(diagnostics.resolution(), AuthResolution::PerRequestHeader);
}

#[test]
fn recording_without_a_scope_diagnostic_is_a_no_op() {
    let mut context = McpContext::default();
    record_auth_resolution(&mut context, "s1", AuthResolution::StaticToken);
    assert!(context.request_scope.get::<MCPAuthDiagnostics>().is_none());
}

#[test]
fn a_second_request_starts_without_the_first_ones_diagnostics() {
    let mut first = McpContext::default();
    first.request_scope.insert(MCPAuthDiagnostics::default());
    record_auth_resolution(&mut first, "s1", AuthResolution::StaticToken);
    let second = McpContext::default();
    assert!(second.request_scope.get::<MCPAuthDiagnostics>().is_none());
}
