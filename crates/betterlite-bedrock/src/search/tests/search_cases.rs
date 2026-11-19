mod bearer;
mod gateway;
mod query;
mod signing;
mod transport;

use rstest::rstest;

#[derive(Clone, Copy)]
enum Outcome {
    ConfiguredGatewayRequired,
    McpRequestHeaders,
    WebsearchToolSuffix,
    QueryListAnd200CharCap,
    HttpsOrLoopbackOnly,
    GatewayBearerTrust,
    GatewayHostSigning,
    GatewaySigningRegion,
}

#[rstest]
#[case::configured_gateway_required(Outcome::ConfiguredGatewayRequired)]
#[case::mcp_request_headers(Outcome::McpRequestHeaders)]
#[case::websearch_tool_suffix(Outcome::WebsearchToolSuffix)]
#[case::query_list_and_200_char_cap(Outcome::QueryListAnd200CharCap)]
#[case::https_or_loopback_only(Outcome::HttpsOrLoopbackOnly)]
#[case::gateway_bearer_trust(Outcome::GatewayBearerTrust)]
#[case::gateway_host_signing(Outcome::GatewayHostSigning)]
#[case::gateway_signing_region(Outcome::GatewaySigningRegion)]
fn search_cases(#[case] outcome: Outcome) {
    match outcome {
        Outcome::ConfiguredGatewayRequired => gateway::assert_configured_gateway_required(),
        Outcome::McpRequestHeaders => gateway::assert_mcp_request_headers(),
        Outcome::WebsearchToolSuffix => gateway::assert_websearch_tool_suffix(),
        Outcome::QueryListAnd200CharCap => query::assert_query_list_and_200_char_cap(),
        Outcome::HttpsOrLoopbackOnly => transport::assert_https_or_loopback_only(),
        Outcome::GatewayBearerTrust => bearer::assert_gateway_bearer_trust(),
        Outcome::GatewayHostSigning => signing::assert_gateway_host_signing(),
        Outcome::GatewaySigningRegion => signing::assert_gateway_signing_region(),
    }
}
