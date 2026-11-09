use std::fs;
use std::path::PathBuf;
use std::str;

use indexmap::IndexMap;
use rstest::rstest;

use crate::redis_cache_admin::info_parse::{
    ClientListError, InfoValue, parse_client_list, parse_info,
};

#[rstest]
#[case::integer("count:42", InfoValue::Int(42))]
#[case::negative("offset:-3", InfoValue::Int(-3))]
#[case::float("ratio:1.5", InfoValue::Float(1.5))]
#[case::dotted_text("redis_version:7.2.4", InfoValue::Text("7.2.4".to_owned()))]
#[case::plain_text("mode:standalone", InfoValue::Text("standalone".to_owned()))]
#[case::comma_list(
    "bind:127.0.0.1,::1",
    InfoValue::List(vec![
        InfoValue::Text("127.0.0.1".to_owned()),
        InfoValue::Text("::1".to_owned()),
    ])
)]
#[case::list_drops_empty_items(
    "nums:1,,2",
    InfoValue::List(vec![InfoValue::Int(1), InfoValue::Int(2)])
)]
#[case::map_of_ints(
    "db0:keys=1,expires=0,avg_ttl=0",
    InfoValue::Map(IndexMap::from([
        ("keys".to_owned(), InfoValue::Int(1)),
        ("expires".to_owned(), InfoValue::Int(0)),
        ("avg_ttl".to_owned(), InfoValue::Int(0)),
    ]))
)]
#[case::map_item_without_equals(
    "flags:a,b=2",
    InfoValue::Map(IndexMap::from([
        ("a".to_owned(), InfoValue::Bool(true)),
        ("b".to_owned(), InfoValue::Int(2)),
    ]))
)]
#[case::map_splits_on_the_last_equals(
    "pair:k=v=1",
    InfoValue::Map(IndexMap::from([("k=v".to_owned(), InfoValue::Int(1))]))
)]
fn a_value_parses_by_its_shape(#[case] line: &str, #[case] expected: InfoValue) {
    let info = parse_info(line);
    assert_eq!(info.len(), 1);
    assert_eq!(info.values().next(), Some(&expected));
}

// python's int() and float() accept these forms, but redis never writes them and i64 does not parse them
#[rstest]
#[case::underscored("1_000")]
#[case::surrounding_space(" 7")]
#[case::past_i64("9223372036854775808")]
fn a_number_python_accepts_but_i64_does_not_stays_text(#[case] value: &str) {
    let info = parse_info(&format!("v:{value}"));
    assert_eq!(info.get("v"), Some(&InfoValue::Text(value.to_owned())));
}

#[test]
fn section_headings_and_empty_lines_are_skipped() {
    let info = parse_info("# Server\n\nredis_version:7.2.4\n\n# Clients\nconnected_clients:3\n");
    assert_eq!(info.len(), 2);
    assert!(!info.contains_key("# Server"));
}

#[test]
fn cmdstat_host_splits_on_its_last_colon() {
    let info = parse_info("cmdstat_host:10.0.0.1:6379\ncmdstat_get:calls=2,usec=9\n");
    assert_eq!(
        info.get("cmdstat_host:10.0.0.1"),
        Some(&InfoValue::Int(6379))
    );
    assert!(!info.contains_key("cmdstat_host"));
    assert_eq!(
        info.get("cmdstat_get"),
        Some(&InfoValue::Map(IndexMap::from([
            ("calls".to_owned(), InfoValue::Int(2)),
            ("usec".to_owned(), InfoValue::Int(9)),
        ])))
    );
}

#[test]
fn repeated_module_lines_collect_under_modules() {
    let info = parse_info("module:name=search,ver=1\nother:1\nmodule:name=ReJSON,ver=2\n");
    let Some(InfoValue::List(modules)) = info.get("modules") else {
        panic!("a modules list");
    };
    assert_eq!(modules.len(), 2);
    assert_eq!(info.keys().collect::<Vec<&String>>(), ["modules", "other"]);
    assert_eq!(
        modules.first(),
        Some(&InfoValue::Map(IndexMap::from([
            ("name".to_owned(), InfoValue::Text("search".to_owned())),
            ("ver".to_owned(), InfoValue::Int(1)),
        ])))
    );
}

#[test]
fn a_line_without_a_colon_goes_under_raw() {
    let info = parse_info("some raw line\nother:1\nanother raw line\n");
    assert_eq!(info.keys().collect::<Vec<&String>>(), ["__raw__", "other"]);
    assert_eq!(
        info.get("__raw__"),
        Some(&InfoValue::List(vec![
            InfoValue::Text("some raw line".to_owned()),
            InfoValue::Text("another raw line".to_owned()),
        ]))
    );
}

#[test]
fn a_repeated_key_keeps_its_first_position_and_last_value() {
    let info = parse_info("first:1\nsecond:2\nfirst:3\n");
    assert_eq!(info.keys().collect::<Vec<&String>>(), ["first", "second"]);
    assert_eq!(info.get("first"), Some(&InfoValue::Int(3)));
}

#[test]
fn client_list_parses_several_clients() {
    let text = "id=3 addr=127.0.0.1:56789 cmd=client|list\nid=4 addr=127.0.0.1:56790 cmd=ping\n";
    let clients = parse_client_list(text).expect("a client list");
    assert_eq!(clients.len(), 2);
    assert_eq!(clients[0].get("id").map(String::as_str), Some("3"));
    assert_eq!(
        clients[0].get("cmd").map(String::as_str),
        Some("client|list")
    );
    assert_eq!(clients[1].get("cmd").map(String::as_str), Some("ping"));
}

#[test]
fn client_list_keeps_a_space_inside_a_value() {
    let text = "id=5 addr=/tmp/redis sock/redis.sock name= age=1\n";
    let clients = parse_client_list(text).expect("a client list");
    assert_eq!(
        clients[0].get("addr").map(String::as_str),
        Some("/tmp/redis sock/redis.sock")
    );
    assert_eq!(clients[0].get("name").map(String::as_str), Some(""));
}

#[test]
fn client_list_returns_an_orphan_token() {
    assert!(matches!(
        parse_client_list("orphan token\n"),
        Err(ClientListError::OrphanToken)
    ));
}

#[test]
fn client_list_of_empty_text_is_empty() {
    let clients = parse_client_list("").expect("an empty client list");
    assert!(clients.is_empty());
}

#[rstest]
fn replays_the_redis_info_corpus(#[files("fuzz/corpus/redis_info/*")] path: PathBuf) {
    let bytes = fs::read(path).expect("a corpus file");
    let Ok(text) = str::from_utf8(&bytes) else {
        return;
    };
    drop(parse_info(text));
    drop(parse_client_list(text));
}
