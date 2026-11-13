use std::str;

use rstest::rstest;
use serde_json::Number;

use super::fake_redis::FakeRedis;
use super::fixtures::command_args;
use crate::arguments::json::Json;
use crate::redis_semantic_cache::error::RedisSemanticError;
use crate::redis_semantic_cache::ports::IndexSchema;
use crate::redis_semantic_cache::{live, schema};

#[rstest]
#[case::empty(vec![], false)]
#[case::one_item(vec![Json::Text("index_name".to_owned())], false)]
#[case::odd_length(vec![Json::Text("a".to_owned()), Json::Null, Json::Null], false)]
#[case::a_single_pair(vec![Json::Text("index_name".to_owned()), Json::Text("idx".to_owned())], true)]
fn info_object_needs_whole_name_value_pairs(#[case] items: Vec<Json>, #[case] valid: bool) {
    let parsed = schema::info_object(&Json::List(items));
    assert_eq!(parsed.is_ok(), valid);
}

#[tokio::test]
async fn info_converts_every_reply_kind() {
    let server = FakeRedis::start().await;
    server.reply(
        "FT.INFO",
        "*5\r\n$3\r\nabc\r\n:7\r\n+simple\r\n$-1\r\n*1\r\n:1\r\n",
    );
    let reply = server
        .connect()
        .await
        .ft_info("idx")
        .await
        .expect("a reply");
    assert_eq!(
        reply,
        Some(Json::List(vec![
            Json::Text("abc".to_owned()),
            Json::Number(Number::from(7_i64)),
            Json::Text("simple".to_owned()),
            Json::Null,
            Json::List(vec![Json::Number(Number::from(1_i64))]),
        ]))
    );
    assert_eq!(
        server.commands("FT.INFO"),
        [vec![b"FT.INFO".to_vec(), b"idx".to_vec()]]
    );
}

#[tokio::test]
async fn info_rejects_a_reply_that_is_not_text_numbers_or_lists() {
    let server = FakeRedis::start().await;
    server.reply("FT.INFO", "+OK\r\n");
    let error = server
        .connect()
        .await
        .ft_info("idx")
        .await
        .expect_err("an unsupported reply kind");
    assert!(matches!(
        error,
        RedisSemanticError::UnexpectedReply { kind: "value" }
    ));
}

#[rstest]
#[case::unknown_index(b"-Unknown index name\r\n", true)]
#[case::no_such_index(b"-no such index\r\n", true)]
#[case::other_error(b"-ERR syntax error\r\n", false)]
#[tokio::test]
async fn info_turns_only_a_missing_index_into_none(#[case] wire: &[u8], #[case] missing: bool) {
    let server = FakeRedis::start().await;
    server.reply("FT.INFO", str::from_utf8(wire).expect("text"));
    let result = server.connect().await.ft_info("idx").await;
    if missing {
        assert_eq!(result.expect("a missing index is not an error"), None);
    } else {
        assert!(matches!(result, Err(RedisSemanticError::Backend(_))));
    }
}

#[tokio::test]
async fn create_and_drop_send_their_commands() {
    let server = FakeRedis::start().await;
    let schema = IndexSchema {
        name: "idx".to_owned(),
        prefix: "idx".to_owned(),
        dimension: 3,
    };
    let index = server.connect().await;
    index.ft_create(&schema).await.expect("created");
    index.ft_dropindex("idx").await.expect("dropped");
    assert_eq!(
        server.commands("FT.CREATE"),
        [command_args(&live::create_command(&schema))]
    );
    assert_eq!(
        server.commands("FT.DROPINDEX"),
        [vec![b"FT.DROPINDEX".to_vec(), b"idx".to_vec()]]
    );
}
