use std::time::Duration;

use redis::Value;
use rstest::rstest;

use super::raw_server::{RawServer, array, bulk};
use super::support::{
    TestGate, build, harness, harness_without_server, settings, settings_with_default,
    transport_for, transport_to,
};
use crate::cache_primitives::protocol::RedisScalar;
use crate::list_operations::error::RedisListOperationsError;
use crate::list_operations::results::{LpopPipelineEntry, LpopResult, decode_reply};
use crate::list_operations::{RedisPipelineLpopOperation, RedisPipelineRpushOperation};
use crate::redis_live_client::tests::fake_server::{FakeServer, Reply};

fn data_commands(server: &FakeServer) -> Vec<String> {
    server
        .commands()
        .into_iter()
        .map(|recorded| recorded.name)
        .filter(|name| matches!(name.as_str(), "MULTI" | "EXEC" | "RPUSH" | "LTRIM" | "LPOP"))
        .collect()
}

fn text(value: &str) -> RedisScalar {
    RedisScalar::Text(value.to_owned())
}

fn push(key: &str, values: &[&str]) -> RedisPipelineRpushOperation {
    RedisPipelineRpushOperation {
        key: key.to_owned(),
        values: values.iter().map(|value| text(value)).collect(),
    }
}

fn pop(key: &str, count: Option<i64>) -> RedisPipelineLpopOperation {
    RedisPipelineLpopOperation {
        key: key.to_owned(),
        count,
    }
}

#[tokio::test]
async fn rpush_appends_values_and_returns_the_new_length() {
    let server = FakeServer::start().await;
    server.script("RPUSH", Reply::Int(4));
    let harness = harness(&server, Some("ns"), "7.0.0");
    let length = harness
        .lists
        .async_rpush("buf", &[text("c"), text("d")])
        .await
        .expect("a push");
    assert_eq!(length, 4);
    assert_eq!(
        server.commands_named("RPUSH")[0].args,
        vec![
            "RPUSH".to_owned(),
            "ns:buf".to_owned(),
            "c".to_owned(),
            "d".to_owned()
        ]
    );
    assert_eq!(
        harness.hooks.entries(),
        vec!["success:async_rpush:Some(\"ns:buf\")".to_owned()]
    );
    assert_eq!(
        harness.gate.entries(),
        vec!["enter:async_rpush".to_owned(), "exit:1".to_owned()]
    );
}

#[rstest]
#[case::text(text("one"), "one")]
#[case::bytes(RedisScalar::Bytes(b"two".to_vec()), "two")]
#[case::float(RedisScalar::Float(1.5_f64), "1.5")]
#[tokio::test]
async fn rpush_sends_each_redis_scalar_as_its_wire_value(
    #[case] value: RedisScalar,
    #[case] expected: &str,
) {
    let server = FakeServer::start().await;
    server.script("RPUSH", Reply::Int(1));
    let harness = harness(&server, None, "7.0.0");
    harness
        .lists
        .async_rpush("buf", &[value])
        .await
        .expect("a push");
    assert_eq!(
        server.commands_named("RPUSH")[0].args,
        vec!["RPUSH".to_owned(), "buf".to_owned(), expected.to_owned()]
    );
}

#[rstest]
#[case::prefixes_the_key(Some("ns"), "buf", "ns:buf")]
#[case::keeps_an_existing_prefix(Some("ns"), "ns:buf", "ns:buf")]
#[case::leaves_it_alone_without_a_namespace(None, "buf", "buf")]
#[tokio::test]
async fn rpush_namespaces_the_key_once(
    #[case] namespace: Option<&str>,
    #[case] key: &str,
    #[case] expected: &str,
) {
    let server = FakeServer::start().await;
    server.script("RPUSH", Reply::Int(1));
    let harness = harness(&server, namespace, "7.0.0");
    harness
        .lists
        .async_rpush(key, &[text("value")])
        .await
        .expect("a push");
    assert_eq!(server.commands_named("RPUSH")[0].args[1], expected);
}

#[tokio::test]
async fn rpush_records_a_command_error_without_exiting_the_gate() {
    let server = FakeServer::start().await;
    server.script(
        "RPUSH",
        Reply::Error("ERR wrong number of arguments for 'rpush' command".to_owned()),
    );
    let harness = harness(&server, Some("ns"), "7.0.0");
    let error = harness
        .lists
        .async_rpush("buf", &[])
        .await
        .expect_err("a failed push");
    assert!(matches!(error, RedisListOperationsError::Live(_)));
    let hooks = harness.hooks.entries();
    assert_eq!(hooks.len(), 1);
    assert!(hooks[0].starts_with("failure:async_rpush:Some(\"ns:buf\"):"));
    let gate = harness.gate.entries();
    assert_eq!(gate[0], "enter:async_rpush");
    assert_eq!(gate[1], format!("recorded:{error}"));
    assert_eq!(gate.len(), 2);
}

#[tokio::test]
async fn rpush_and_trim_pushes_and_trims_in_one_transaction() {
    let server = FakeServer::start().await;
    server.script("RPUSH", Reply::Int(4));
    server.script("LTRIM", Reply::Simple("OK".to_owned()));
    let harness = harness(&server, Some("ns"), "7.0.0");
    let length = harness
        .lists
        .async_rpush_and_trim("buf", &[text("c"), text("d")], 3)
        .await
        .expect("a push");
    assert_eq!(length, 4);
    assert_eq!(
        data_commands(&server),
        vec![
            "MULTI".to_owned(),
            "RPUSH".to_owned(),
            "LTRIM".to_owned(),
            "EXEC".to_owned()
        ]
    );
    assert_eq!(
        server.commands_named("LTRIM")[0].args,
        vec![
            "LTRIM".to_owned(),
            "ns:buf".to_owned(),
            "-3".to_owned(),
            "-1".to_owned()
        ]
    );
    assert_eq!(
        harness.hooks.entries(),
        vec!["success:async_rpush_and_trim:Some(\"ns:buf\")".to_owned()]
    );
}

#[rstest]
#[case::zero_keeps_everything(0_i64, "0")]
#[case::negative_drops_from_the_front(-2_i64, "2")]
#[tokio::test]
async fn rpush_and_trim_negates_the_kept_length(#[case] max_len: i64, #[case] start: &str) {
    let server = FakeServer::start().await;
    server.script("RPUSH", Reply::Int(2));
    server.script("LTRIM", Reply::Simple("OK".to_owned()));
    let harness = harness(&server, None, "7.0.0");
    harness
        .lists
        .async_rpush_and_trim("buf", &[text("c")], max_len)
        .await
        .expect("a push");
    assert_eq!(
        server.commands_named("LTRIM")[0].args,
        vec![
            "LTRIM".to_owned(),
            "buf".to_owned(),
            start.to_owned(),
            "-1".to_owned()
        ]
    );
}

#[tokio::test]
async fn rpush_and_trim_rejects_a_length_that_cannot_be_negated() {
    let server = FakeServer::start().await;
    let harness = harness(&server, None, "7.0.0");
    let error = harness
        .lists
        .async_rpush_and_trim("buf", &[text("c")], i64::MIN)
        .await
        .expect_err("an impossible trim");
    assert!(matches!(
        error,
        RedisListOperationsError::TrimLength { max_len: i64::MIN }
    ));
    assert!(server.commands().is_empty());
}

#[tokio::test]
async fn rpush_and_trim_propagates_a_command_error() {
    let server = FakeServer::start().await;
    server.script("RPUSH", Reply::Int(4));
    server.script("LTRIM", Reply::Error("ERR trim failed".to_owned()));
    let harness = harness(&server, Some("ns"), "7.0.0");
    let error = harness
        .lists
        .async_rpush_and_trim("buf", &[text("c")], 3)
        .await
        .expect_err("a failed trim");
    assert!(matches!(error, RedisListOperationsError::Live(_)));
    assert_eq!(harness.hooks.entries().len(), 1);
    assert_eq!(harness.gate.entries()[0], "enter:async_rpush_and_trim");
    assert_eq!(harness.gate.entries()[1], format!("recorded:{error}"));
}

#[tokio::test]
async fn rpush_pipeline_returns_one_length_per_operation() {
    let server = FakeServer::start().await;
    server.script("RPUSH", Reply::Int(3));
    server.script("RPUSH", Reply::Int(5));
    let harness = harness(&server, Some("ns"), "7.0.0");
    let lengths = harness
        .lists
        .async_rpush_pipeline(&[push("a", &["x"]), push("b", &["y"])])
        .await
        .expect("a pipeline");
    assert_eq!(lengths, vec![3, 5]);
    let pushes = server.commands_named("RPUSH");
    assert_eq!(
        pushes[0].args,
        vec!["RPUSH".to_owned(), "ns:a".to_owned(), "x".to_owned()]
    );
    assert_eq!(
        pushes[1].args,
        vec!["RPUSH".to_owned(), "ns:b".to_owned(), "y".to_owned()]
    );
    assert!(!data_commands(&server).contains(&"MULTI".to_owned()));
    assert_eq!(
        harness.hooks.entries(),
        vec!["success:async_rpush_pipeline:None".to_owned()]
    );
}

#[tokio::test]
async fn rpush_pipeline_sends_nothing_for_an_empty_list() {
    let server = FakeServer::start().await;
    let harness = harness(&server, Some("ns"), "7.0.0");
    let lengths = harness
        .lists
        .async_rpush_pipeline(&[])
        .await
        .expect("an empty pipeline");
    assert!(lengths.is_empty());
    assert_eq!(
        harness.gate.entries(),
        vec!["enter:async_rpush_pipeline".to_owned(), "exit:1".to_owned()]
    );
    assert!(harness.hooks.entries().is_empty());
    assert!(server.commands().is_empty());
    assert_eq!(server.connections(), 0);
}

#[tokio::test]
async fn rpush_pipeline_propagates_the_first_error() {
    let server = FakeServer::start().await;
    server.script("RPUSH", Reply::Int(1));
    server.script("RPUSH", Reply::Error("ERR second push".to_owned()));
    let harness = harness(&server, None, "7.0.0");
    let error = harness
        .lists
        .async_rpush_pipeline(&[push("a", &["x"]), push("b", &["y"])])
        .await
        .expect_err("a failed pipeline");
    assert!(matches!(error, RedisListOperationsError::Live(_)));
    assert_eq!(harness.hooks.entries().len(), 1);
    assert_eq!(
        harness.gate.entries(),
        vec![
            "enter:async_rpush_pipeline".to_owned(),
            format!("recorded:{error}")
        ]
    );
}

#[tokio::test]
async fn lpop_returns_one_decoded_value_for_a_single_pop() {
    let server = FakeServer::start().await;
    server.script("LPOP", Reply::Bulk("first".to_owned()));
    let harness = harness(&server, Some("ns"), "7.2.0");
    let result = harness
        .lists
        .async_lpop("queue", None)
        .await
        .expect("a pop");
    assert_eq!(result, LpopResult::Item("first".to_owned()));
    assert_eq!(
        server.commands_named("LPOP")[0].args,
        vec!["LPOP".to_owned(), "ns:queue".to_owned()]
    );
    assert_eq!(
        harness.hooks.entries(),
        vec!["success:async_lpop:Some(\"ns:queue\")".to_owned()]
    );
}

#[tokio::test]
async fn lpop_returns_missing_for_an_empty_list() {
    let server = FakeServer::start().await;
    server.script("LPOP", Reply::Nil);
    let harness = harness(&server, Some("ns"), "7.0.0");
    let result = harness
        .lists
        .async_lpop("queue", None)
        .await
        .expect("a pop");
    assert_eq!(result, LpopResult::Missing);
}

#[rstest]
#[case::dotted_seven("7.0.0")]
#[case::bare_seven("7")]
#[case::eight("8.2.1")]
#[tokio::test]
async fn lpop_uses_one_bulk_pop_from_redis_seven(#[case] version: &str) {
    let server = FakeServer::start().await;
    server.script(
        "LPOP",
        Reply::Array(vec![
            Reply::Bulk("a".to_owned()),
            Reply::Bulk("b".to_owned()),
        ]),
    );
    let harness = harness(&server, Some("ns"), version);
    let result = harness
        .lists
        .async_lpop("queue", Some(2))
        .await
        .expect("a pop");
    assert_eq!(
        result,
        LpopResult::Items(vec!["a".to_owned(), "b".to_owned()])
    );
    assert_eq!(
        server.commands_named("LPOP")[0].args,
        vec!["LPOP".to_owned(), "ns:queue".to_owned(), "2".to_owned()]
    );
}

#[rstest]
#[case::dotted_six("6.2.0")]
#[case::bare_six("6")]
#[case::five("5.0.7")]
#[tokio::test]
async fn lpop_loops_single_pops_below_redis_seven(#[case] version: &str) {
    let server = FakeServer::start().await;
    server.script("LPOP", Reply::Bulk("a".to_owned()));
    server.script("LPOP", Reply::Bulk("b".to_owned()));
    let harness = harness(&server, Some("ns"), version);
    let result = harness
        .lists
        .async_lpop("queue", Some(2))
        .await
        .expect("a pop");
    assert_eq!(
        result,
        LpopResult::Items(vec!["a".to_owned(), "b".to_owned()])
    );
    let pops = server.commands_named("LPOP");
    assert_eq!(pops.len(), 2);
    assert_eq!(pops[0].args, vec!["LPOP".to_owned(), "ns:queue".to_owned()]);
    assert_eq!(pops[1].args, vec!["LPOP".to_owned(), "ns:queue".to_owned()]);
}

#[tokio::test]
async fn lpop_below_redis_seven_sends_one_pop_without_a_count() {
    let server = FakeServer::start().await;
    server.script("LPOP", Reply::Bulk("only".to_owned()));
    let harness = harness(&server, None, "6.2.0");
    let result = harness
        .lists
        .async_lpop("queue", None)
        .await
        .expect("a pop");
    assert_eq!(result, LpopResult::Item("only".to_owned()));
    assert_eq!(server.commands_named("LPOP").len(), 1);
}

#[tokio::test]
async fn lpop_below_redis_seven_answers_an_empty_list_with_no_items() {
    let server = FakeServer::start().await;
    server.script("LPOP", Reply::Nil);
    server.script("LPOP", Reply::Nil);
    let harness = harness(&server, None, "6.2.0");
    let result = harness
        .lists
        .async_lpop("queue", Some(2))
        .await
        .expect("a pop");
    assert_eq!(result, LpopResult::Items(Vec::new()));
}

#[tokio::test]
async fn lpop_falls_back_to_the_default_major_version_for_an_unknown_version() {
    let server = FakeServer::start().await;
    server.script(
        "LPOP",
        Reply::Array(vec![
            Reply::Bulk("a".to_owned()),
            Reply::Bulk("b".to_owned()),
        ]),
    );
    let harness = build(
        settings_with_default(None, "Unknown", 7),
        transport_for(&server),
        TestGate::never(),
    );
    let result = harness
        .lists
        .async_lpop("queue", Some(2))
        .await
        .expect("a pop");
    assert_eq!(
        result,
        LpopResult::Items(vec!["a".to_owned(), "b".to_owned()])
    );
    assert_eq!(server.commands_named("LPOP")[0].args.len(), 3);
}

#[tokio::test]
async fn lpop_uses_a_six_default_for_an_unknown_version() {
    let server = FakeServer::start().await;
    server.script("LPOP", Reply::Bulk("a".to_owned()));
    server.script("LPOP", Reply::Bulk("b".to_owned()));
    let harness = build(
        settings_with_default(None, "Unknown", 6),
        transport_for(&server),
        TestGate::never(),
    );
    let result = harness
        .lists
        .async_lpop("queue", Some(2))
        .await
        .expect("a pop");
    assert_eq!(
        result,
        LpopResult::Items(vec!["a".to_owned(), "b".to_owned()])
    );
    let pops = server.commands_named("LPOP");
    assert_eq!(pops.len(), 2);
    assert_eq!(pops[0].args, vec!["LPOP".to_owned(), "queue".to_owned()]);
}

#[test]
fn lpop_keeps_the_original_bytes_when_utf8_decoding_fails() {
    let single = decode_reply(&Value::BulkString(vec![0xff])).expect("a reply");
    assert_eq!(single, LpopResult::ItemBytes(vec![0xff]));
    let list = decode_reply(&Value::Array(vec![
        Value::BulkString(b"ok".to_vec()),
        Value::BulkString(vec![0xff]),
    ]))
    .expect("a reply");
    assert_eq!(
        list,
        LpopResult::ItemsBytes(vec![b"ok".to_vec(), vec![0xff]])
    );
}

#[tokio::test]
async fn lpop_records_a_command_error_without_exiting_the_gate() {
    let server = FakeServer::start().await;
    server.script("LPOP", Reply::Error("ERR pop failed".to_owned()));
    let harness = harness(&server, Some("ns"), "6.2.0");
    let error = harness
        .lists
        .async_lpop("queue", Some(1))
        .await
        .expect_err("a failed pop");
    assert!(matches!(error, RedisListOperationsError::Live(_)));
    assert_eq!(harness.hooks.entries().len(), 1);
    assert_eq!(
        harness.gate.entries(),
        vec!["enter:async_lpop".to_owned(), format!("recorded:{error}")]
    );
}

#[tokio::test]
async fn lpop_pipeline_keeps_each_operation_isolated() {
    let server = FakeServer::start().await;
    server.script(
        "LPOP",
        Reply::Array(vec![
            Reply::Bulk("a1".to_owned()),
            Reply::Bulk("a2".to_owned()),
        ]),
    );
    server.script("LPOP", Reply::Nil);
    server.script("LPOP", Reply::Array(vec![Reply::Bulk("c1".to_owned())]));
    let harness = harness(&server, Some("ns"), "7.0.0");
    let entries = harness
        .lists
        .async_lpop_pipeline(&[pop("a", Some(2)), pop("b", Some(2)), pop("c", Some(1))])
        .await
        .expect("a pipeline");
    assert_eq!(
        entries,
        vec![
            LpopPipelineEntry::Items(vec!["a1".to_owned(), "a2".to_owned()]),
            LpopPipelineEntry::Missing,
            LpopPipelineEntry::Items(vec!["c1".to_owned()]),
        ]
    );
    let pops = server.commands_named("LPOP");
    assert_eq!(
        pops[0].args,
        vec!["LPOP".to_owned(), "ns:a".to_owned(), "2".to_owned()]
    );
    assert_eq!(
        pops[1].args,
        vec!["LPOP".to_owned(), "ns:b".to_owned(), "2".to_owned()]
    );
    assert_eq!(
        pops[2].args,
        vec!["LPOP".to_owned(), "ns:c".to_owned(), "1".to_owned()]
    );
    assert!(!data_commands(&server).contains(&"MULTI".to_owned()));
}

#[tokio::test]
async fn lpop_pipeline_regroups_flat_pops_below_redis_seven() {
    let server = FakeServer::start().await;
    server.script("LPOP", Reply::Bulk("a1".to_owned()));
    server.script("LPOP", Reply::Bulk("a2".to_owned()));
    server.script("LPOP", Reply::Nil);
    server.script("LPOP", Reply::Nil);
    server.script("LPOP", Reply::Bulk("c1".to_owned()));
    let harness = harness(&server, Some("ns"), "6.2.0");
    let entries = harness
        .lists
        .async_lpop_pipeline(&[pop("a", Some(2)), pop("b", Some(2)), pop("c", Some(1))])
        .await
        .expect("a pipeline");
    assert_eq!(
        entries,
        vec![
            LpopPipelineEntry::Items(vec!["a1".to_owned(), "a2".to_owned()]),
            LpopPipelineEntry::Missing,
            LpopPipelineEntry::Items(vec!["c1".to_owned()]),
        ]
    );
    let pops = server.commands_named("LPOP");
    assert_eq!(pops.len(), 5);
    assert_eq!(pops[0].args, vec!["LPOP".to_owned(), "ns:a".to_owned()]);
    assert_eq!(pops[1].args, vec!["LPOP".to_owned(), "ns:a".to_owned()]);
    assert_eq!(pops[2].args, vec!["LPOP".to_owned(), "ns:b".to_owned()]);
    assert_eq!(pops[3].args, vec!["LPOP".to_owned(), "ns:b".to_owned()]);
    assert_eq!(pops[4].args, vec!["LPOP".to_owned(), "ns:c".to_owned()]);
}

#[tokio::test]
async fn lpop_pipeline_below_redis_seven_uses_one_pop_for_a_missing_count() {
    let server = FakeServer::start().await;
    server.script("LPOP", Reply::Bulk("a1".to_owned()));
    server.script("LPOP", Reply::Bulk("c1".to_owned()));
    let harness = harness(&server, None, "6.2.0");
    let entries = harness
        .lists
        .async_lpop_pipeline(&[pop("a", None), pop("b", Some(0)), pop("c", Some(1))])
        .await
        .expect("a pipeline");
    assert_eq!(
        entries,
        vec![
            LpopPipelineEntry::Items(vec!["a1".to_owned()]),
            LpopPipelineEntry::Missing,
            LpopPipelineEntry::Items(vec!["c1".to_owned()]),
        ]
    );
    let pops = server.commands_named("LPOP");
    assert_eq!(pops.len(), 2);
    assert_eq!(pops[0].args, vec!["LPOP".to_owned(), "a".to_owned()]);
    assert_eq!(pops[1].args, vec!["LPOP".to_owned(), "c".to_owned()]);
}

#[tokio::test]
async fn lpop_pipeline_below_redis_seven_pops_nothing_for_a_negative_count() {
    let server = FakeServer::start().await;
    server.script("LPOP", Reply::Bulk("b1".to_owned()));
    let harness = harness(&server, None, "6.2.0");
    let entries = harness
        .lists
        .async_lpop_pipeline(&[pop("a", Some(-1)), pop("b", Some(1))])
        .await
        .expect("a pipeline");
    assert_eq!(
        entries,
        vec![
            LpopPipelineEntry::Missing,
            LpopPipelineEntry::Items(vec!["b1".to_owned()]),
        ]
    );
    let pops = server.commands_named("LPOP");
    assert_eq!(pops.len(), 1);
    assert_eq!(pops[0].args, vec!["LPOP".to_owned(), "b".to_owned()]);
}

#[tokio::test]
async fn lpop_pipeline_regroups_a_partly_empty_group() {
    let server = FakeServer::start().await;
    server.script("LPOP", Reply::Bulk("a1".to_owned()));
    server.script("LPOP", Reply::Nil);
    server.script("LPOP", Reply::Bulk("a3".to_owned()));
    let harness = harness(&server, None, "6.2.0");
    let entries = harness
        .lists
        .async_lpop_pipeline(&[pop("a", Some(3))])
        .await
        .expect("a pipeline");
    assert_eq!(
        entries,
        vec![LpopPipelineEntry::Items(vec![
            "a1".to_owned(),
            "a3".to_owned()
        ])]
    );
}

#[rstest]
#[case::redis_seven_bulk("7.0.0", vec![array(&[vec![0xff], b"ok".to_vec()])])]
#[case::redis_six_flat("6.2.0", vec![bulk(&[0xff]), bulk(b"ok")])]
#[tokio::test]
async fn lpop_pipeline_keeps_the_original_bytes(
    #[case] version: &str,
    #[case] replies: Vec<Vec<u8>>,
) {
    let server = RawServer::start(replies).expect("a raw server");
    let harness = build(
        settings(None, version),
        transport_to(&server.host(), server.port()),
        TestGate::never(),
    );
    let entries = harness
        .lists
        .async_lpop_pipeline(&[pop("a", Some(2))])
        .await
        .expect("a pipeline");
    assert_eq!(
        entries,
        vec![LpopPipelineEntry::ItemsBytes(vec![
            vec![0xff],
            b"ok".to_vec()
        ])]
    );
    assert_eq!(server.pending(), 0);
}

#[tokio::test]
async fn lpop_keeps_the_original_bytes_from_the_old_redis_loop_too() {
    let server = RawServer::start(vec![bulk(&[0xff]), bulk(b"ok")]).expect("a raw server");
    let harness = build(
        settings(None, "6.2.0"),
        transport_to(&server.host(), server.port()),
        TestGate::never(),
    );
    let result = harness
        .lists
        .async_lpop("queue", Some(2))
        .await
        .expect("a pop");
    assert_eq!(
        result,
        LpopResult::ItemsBytes(vec![vec![0xff], b"ok".to_vec()])
    );
}

#[tokio::test]
async fn lpop_pipeline_bulk_mode_answers_an_empty_array_with_missing() {
    let server = FakeServer::start().await;
    server.script("LPOP", Reply::Array(Vec::new()));
    let harness = harness(&server, None, "7.0.0");
    let entries = harness
        .lists
        .async_lpop_pipeline(&[pop("a", Some(2))])
        .await
        .expect("a pipeline");
    assert_eq!(entries, vec![LpopPipelineEntry::Missing]);
}

#[tokio::test]
async fn lpop_pipeline_bulk_mode_keeps_a_bare_bulk_pop() {
    let server = FakeServer::start().await;
    server.script("LPOP", Reply::Bulk("solo".to_owned()));
    let harness = harness(&server, None, "7.0.0");
    let entries = harness
        .lists
        .async_lpop_pipeline(&[pop("a", None)])
        .await
        .expect("a pipeline");
    assert_eq!(
        entries,
        vec![LpopPipelineEntry::Items(vec!["solo".to_owned()])]
    );
}

#[tokio::test]
async fn lpop_pipeline_sends_nothing_for_an_empty_list() {
    let server = FakeServer::start().await;
    let harness = harness(&server, None, "7.0.0");
    let entries = harness
        .lists
        .async_lpop_pipeline(&[])
        .await
        .expect("an empty pipeline");
    assert!(entries.is_empty());
    assert_eq!(
        harness.gate.entries(),
        vec!["enter:async_lpop_pipeline".to_owned(), "exit:1".to_owned()]
    );
    assert!(harness.hooks.entries().is_empty());
    assert!(server.commands().is_empty());
    assert_eq!(server.connections(), 0);
}

#[tokio::test]
async fn lpop_pipeline_propagates_a_command_error() {
    let server = FakeServer::start().await;
    server.script("LPOP", Reply::Error("ERR first pop".to_owned()));
    server.script("LPOP", Reply::Bulk("b1".to_owned()));
    let harness = harness(&server, None, "7.0.0");
    let error = harness
        .lists
        .async_lpop_pipeline(&[pop("a", Some(1)), pop("b", Some(1))])
        .await
        .expect_err("a failed pipeline");
    assert!(matches!(error, RedisListOperationsError::Live(_)));
    assert_eq!(harness.hooks.entries().len(), 1);
    assert_eq!(
        harness.gate.entries(),
        vec![
            "enter:async_lpop_pipeline".to_owned(),
            format!("recorded:{error}")
        ]
    );
}

#[tokio::test]
async fn an_open_breaker_rejects_every_list_operation_without_a_command() {
    let server = FakeServer::start().await;
    let harness = build(
        settings(Some("ns"), "7.0.0"),
        transport_for(&server),
        TestGate::always(),
    );
    let lists = &harness.lists;
    let pushed = lists.async_rpush("buf", &[text("v")]).await;
    assert!(matches!(
        pushed,
        Err(RedisListOperationsError::BreakerOpen(_))
    ));
    let trimmed = lists.async_rpush_and_trim("buf", &[text("v")], 2).await;
    assert!(matches!(
        trimmed,
        Err(RedisListOperationsError::BreakerOpen(_))
    ));
    let bulk = lists.async_rpush_pipeline(&[push("buf", &["v"])]).await;
    assert!(matches!(
        bulk,
        Err(RedisListOperationsError::BreakerOpen(_))
    ));
    let popped = lists.async_lpop("buf", Some(1)).await;
    assert!(matches!(
        popped,
        Err(RedisListOperationsError::BreakerOpen(_))
    ));
    let drained = lists.async_lpop_pipeline(&[pop("buf", Some(1))]).await;
    assert!(matches!(
        drained,
        Err(RedisListOperationsError::BreakerOpen(_))
    ));
    assert!(server.commands().is_empty());
    assert_eq!(server.connections(), 0);
    assert!(harness.hooks.entries().is_empty());
}

#[tokio::test]
async fn a_rejected_connection_fires_the_failure_hook_and_records_it() {
    let harness = harness_without_server(Some("ns"));
    let error = harness
        .lists
        .async_rpush("buf", &[text("v")])
        .await
        .expect_err("a refused connection");
    assert!(matches!(
        error,
        RedisListOperationsError::ClientReuse(_) | RedisListOperationsError::Live(_)
    ));
    let hooks = harness.hooks.entries();
    assert_eq!(hooks.len(), 1);
    assert!(hooks[0].starts_with("failure:async_rpush:Some(\"ns:buf\"):"));
    assert_eq!(
        harness.gate.entries(),
        vec!["enter:async_rpush".to_owned(), format!("recorded:{error}")]
    );
}

#[tokio::test]
async fn hooks_carry_the_operation_name_and_a_measured_duration() {
    let server = FakeServer::start().await;
    server.script("RPUSH", Reply::Int(1));
    let harness = harness(&server, None, "7.0.0");
    harness
        .lists
        .async_rpush("buf", &[text("v")])
        .await
        .expect("a push");
    let durations = harness.durations();
    assert_eq!(durations.len(), 1);
    assert!(durations[0] > Duration::ZERO);
    assert_eq!(
        harness.hooks.entries(),
        vec!["success:async_rpush:Some(\"buf\")".to_owned()]
    );
}

#[tokio::test]
async fn a_failed_hook_carries_a_measured_duration_too() {
    let server = FakeServer::start().await;
    server.script("LPOP", Reply::Error("ERR pop failed".to_owned()));
    let harness = harness(&server, None, "7.0.0");
    let _error = harness
        .lists
        .async_lpop("queue", Some(1))
        .await
        .expect_err("a failed pop");
    let durations = harness.durations();
    assert_eq!(durations.len(), 1);
    assert!(durations[0] > Duration::ZERO);
    assert!(harness.hooks.entries()[0].starts_with("failure:async_lpop:Some(\"queue\"):"));
}

#[tokio::test]
async fn an_rpush_and_trim_error_on_the_push_records_once() {
    let server = FakeServer::start().await;
    server.script("RPUSH", Reply::Error("ERR push failed".to_owned()));
    server.script("LTRIM", Reply::Simple("OK".to_owned()));
    let harness = harness(&server, None, "7.0.0");
    let error = harness
        .lists
        .async_rpush_and_trim("buf", &[text("c")], 3)
        .await
        .expect_err("a failed push");
    assert!(matches!(error, RedisListOperationsError::Live(_)));
    let gate = harness.gate.entries();
    assert_eq!(gate.len(), 2);
    assert_eq!(gate[0], "enter:async_rpush_and_trim");
    assert_eq!(gate[1], format!("recorded:{error}"));
}
