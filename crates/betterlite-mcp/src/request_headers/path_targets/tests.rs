#![cfg(test)]

use std::fs;
use std::path::PathBuf;
use std::str;

use rstest::rstest;

use super::{cold_start_server_names, resolve_target_server_names, target_names_from_path};

fn names(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

#[rstest]
#[case::sse("/mcp/sse", vec![])]
#[case::sse_trailing("/mcp/sse/", vec![])]
#[case::sse_messages("/mcp/sse/messages", vec![])]
#[case::sse_messages_trailing("/mcp/sse/messages/", vec![])]
#[case::aggregate("/mcp", vec![])]
#[case::aggregate_trailing("/mcp/", vec![])]
#[case::empty("", vec![])]
#[case::slash("/", vec![])]
#[case::relative("mcp/a", vec![])]
#[case::uppercase("/MCP/a", vec![])]
#[case::server_first("/foo/mcp", vec!["foo"])]
#[case::server_first_tail("/foo/mcp/bar", vec!["foo"])]
#[case::server_first_query("/foo/mcp?x=1", vec![])]
#[case::single("/mcp/a", vec!["a"])]
#[case::single_with_tail("/mcp/a/b", vec!["a/b"])]
#[case::single_long_tail("/mcp/a/b/c", vec!["a/b"])]
#[case::single_query("/mcp/x?y=1", vec!["x"])]
#[case::single_fragment("/mcp/x#f", vec!["x"])]
#[case::single_query_fragment("/mcp/a?x=1#f", vec!["a"])]
#[case::single_empty_query("/mcp/a/b?", vec!["a/b"])]
#[case::single_tail_query("/mcp/a/b/c?x", vec!["a/b"])]
#[case::single_space("/mcp/a b", vec!["a b"])]
#[case::inner_mcp("/mcp/mcp/foo", vec!["mcp/foo"])]
#[case::double_slash("/mcp//x", vec!["/x"])]
#[case::csv("/mcp/a,b", vec!["a", "b"])]
#[case::csv_tail("/mcp/a,b/tail", vec!["a", "b"])]
#[case::csv_long_tail("/mcp/a,b/tail/more", vec!["a", "b"])]
#[case::csv_trailing_slash("/mcp/a,b/", vec!["a", "b/"])]
#[case::csv_empty_segment("/mcp/a,b//c", vec!["a", "b/"])]
#[case::csv_second_comma("/mcp/a,b/c,d", vec!["a", "b/c", "d"])]
#[case::csv_slash_before_comma("/mcp/foo/bar,baz", vec!["foo/bar", "baz"])]
fn target_names_from_path_matches_litellm(#[case] path: &str, #[case] expected: Vec<&str>) {
    assert_eq!(target_names_from_path(path), names(&expected));
}

#[test]
fn path_targets_win_over_the_header() {
    let header = names(&["other"]);
    assert_eq!(
        resolve_target_server_names("/foo/mcp", Some(&header)),
        names(&["foo"])
    );
}

#[test]
fn the_header_is_used_when_the_path_names_no_target() {
    let header = names(&["one", "two"]);
    assert_eq!(
        resolve_target_server_names("/tools", Some(&header)),
        names(&["one", "two"])
    );
    assert_eq!(
        resolve_target_server_names("/tools", Some(&[])),
        Vec::<String>::new()
    );
    assert_eq!(
        resolve_target_server_names("/tools", None),
        Vec::<String>::new()
    );
}

#[rstest]
#[case::single_target("/foo/mcp", vec![], vec!["foo"])]
#[case::single_target_confirmed("/foo/mcp", vec!["foo"], vec!["foo"])]
#[case::multi_target("/mcp/a,b", vec![], vec![])]
#[case::no_target("/tools", vec![], vec![])]
#[case::header_adds_a_server("/foo/mcp", vec!["foo", "other"], vec![])]
fn cold_start_server_names_matches_litellm(
    #[case] path: &str,
    #[case] header: Vec<&str>,
    #[case] expected: Vec<&str>,
) {
    let header_servers: Option<Vec<String>> = (!header.is_empty()).then(|| names(&header));
    assert_eq!(
        cold_start_server_names(path, header_servers.as_deref()).as_deref(),
        expected.first().copied()
    );
}

#[rstest]
fn replays_the_request_path_targets_corpus(
    #[files("fuzz/corpus/request_path_targets/*")] path: PathBuf,
) {
    let bytes = fs::read(path).unwrap();
    if let Ok(text) = str::from_utf8(&bytes) {
        drop(target_names_from_path(text));
    }
}
