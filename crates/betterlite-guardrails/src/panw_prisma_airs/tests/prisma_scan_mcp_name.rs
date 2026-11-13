use std::collections::BTreeMap;

use rstest::rstest;

use crate::prisma_airs_masking::mcp::{McpServer, get_mcp_server_name, parse_mcp_request};

fn servers() -> BTreeMap<String, McpServer> {
    BTreeMap::from([
        (
            "srv-1".to_owned(),
            McpServer {
                alias: Some("alias-1".to_owned()),
                server_name: Some("name-1".to_owned()),
                name: None,
                server_id: Some("srv-1".to_owned()),
            },
        ),
        (
            "srv-2".to_owned(),
            McpServer {
                alias: Some(String::new()),
                server_name: None,
                name: None,
                server_id: None,
            },
        ),
    ])
}

fn tools() -> BTreeMap<String, String> {
    BTreeMap::from([("search".to_owned(), "from-tool".to_owned())])
}

#[rstest]
#[case::explicit_mcp_server_name(
    r#"{"mcp_server_name":"explicit","server_name":"other"}"#,
    "explicit"
)]
#[case::server_name(r#"{"server_name":"named"}"#, "named")]
#[case::registry_alias(r#"{"server_id":"srv-1"}"#, "alias-1")]
#[case::registry_entry_without_names(r#"{"server_id":"srv-2"}"#, "unknown")]
#[case::tool_mapping_when_the_id_is_unknown(r#"{"server_id":"missing"}"#, "from-tool")]
#[case::tool_mapping("{}", "from-tool")]
#[case::empty_names_fall_through(r#"{"mcp_server_name":"","server_name":""}"#, "from-tool")]
fn server_name_follows_the_documented_order(#[case] body: &str, #[case] expected: &str) {
    let request = parse_mcp_request(body.as_bytes()).expect("request parses");
    assert_eq!(
        get_mcp_server_name(&request, &servers(), &tools(), "search"),
        expected
    );
}

#[test]
fn an_unmapped_tool_gets_unknown() {
    let request = parse_mcp_request(b"{}").expect("request parses");
    assert_eq!(
        get_mcp_server_name(&request, &servers(), &tools(), "other"),
        "unknown"
    );
}
