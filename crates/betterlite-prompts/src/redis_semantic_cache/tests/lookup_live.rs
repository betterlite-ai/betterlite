use rstest::rstest;
use serde_json::Number;

use super::fake_redis::FakeRedis;
use super::fixtures::command_args;
use crate::arguments::json::Json;
use crate::redis_semantic_cache::ports::SemanticSearchQuery;
use crate::redis_semantic_cache::{live, query};

#[rstest]
#[case::unknown_index_name(b"-Unknown index name\r\n".as_slice(), true)]
#[case::no_such_index(b"-no such index\r\n".as_slice(), true)]
#[case::err_prefixed(b"-ERR unknown index name\r\n".as_slice(), true)]
#[case::other_error(b"-ERR syntax error\r\n".as_slice(), false)]
fn lookup_server_replies_for_a_missing_index_are_recognised(
    #[case] wire: &[u8],
    #[case] missing: bool,
) {
    let parsed = redis::parse_redis_value(wire).expect("a parsed reply");
    let error = parsed.extract_error().expect_err("a server error");
    assert_eq!(live::is_missing_index_error(&error), missing);
}

#[test]
fn lookup_search_reply_accepts_a_numeric_distance() {
    let reply = Json::List(vec![
        Json::Number(Number::from(1_i64)),
        Json::Text("idx:entry".to_owned()),
        Json::List(vec![
            Json::Text("vector_distance".to_owned()),
            Json::Number(Number::from_f64(0.25).expect("a number")),
        ]),
    ]);
    let hits = query::parse_search_reply(&reply).expect("a search reply");
    assert_eq!(hits[0].vector_distance, Some(0.25_f64));
}

#[tokio::test]
async fn search_sends_the_command_and_parses_the_hits() {
    let server = FakeRedis::start().await;
    server.reply(
        "FT.SEARCH",
        "*3\r\n:1\r\n$9\r\nidx:entry\r\n*4\r\n$8\r\nresponse\r\n$2\r\n{}\r\n$15\r\nvector_distance\r\n$3\r\n0.1\r\n",
    );
    let query = SemanticSearchQuery {
        vector: vec![1.0_f32, 0.5_f32],
        distance_threshold: 0.25,
        cache_key: "scope".to_owned(),
    };
    let hits = server
        .connect()
        .await
        .ft_search("idx", &query)
        .await
        .expect("hits");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].response.as_deref(), Some("{}"));
    assert_eq!(hits[0].vector_distance, Some(0.1_f64));
    assert_eq!(
        server.commands("FT.SEARCH"),
        [command_args(&live::search_command("idx", &query))]
    );
}
