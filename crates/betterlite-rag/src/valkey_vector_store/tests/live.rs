use std::time::Duration;

use betterlite_redis::redis_live_client::LiveClientFactory;
use redis::Value;
use rstest::rstest;

use super::fake_server::FakeValkey;
use super::fakes::document;
use crate::valkey_vector_store::error::ValkeyError;
use crate::valkey_vector_store::live::LiveValkeyConnector;
use crate::valkey_vector_store::ports::{SocketTimeouts, ValkeyConnector as _, ValkeySearchParams};
use crate::valkey_vector_store::query::ValkeyKnnQuery;
use crate::valkey_vector_store::wire;

const REPLY: &str = "*3\r\n:1\r\n$5\r\ndoc-a\r\n*4\r\n$4\r\ntext\r\n$5\r\nfirst\r\n$15\r\nvector_distance\r\n$4\r\n0.25\r\n";

fn query() -> ValkeyKnnQuery {
    ValkeyKnnQuery {
        expression: "*=>[KNN 10 @embedding $vec AS vector_distance]".to_owned(),
        return_fields: vec!["text".to_owned(), "vector_distance".to_owned()],
        offset: 0,
        count: 10,
        dialect: 2,
    }
}

fn search_params() -> ValkeySearchParams {
    let mut params = ValkeySearchParams::new();
    params.insert("vec".to_owned(), vec![0, 0, 0, 63]);
    params
}

fn expected_search_arguments() -> Vec<Vec<u8>> {
    vec![
        b"FT.SEARCH".to_vec(),
        b"vs_index".to_vec(),
        b"*=>[KNN 10 @embedding $vec AS vector_distance]".to_vec(),
        b"RETURN".to_vec(),
        b"2".to_vec(),
        b"text".to_vec(),
        b"vector_distance".to_vec(),
        b"DIALECT".to_vec(),
        b"2".to_vec(),
        b"LIMIT".to_vec(),
        b"0".to_vec(),
        b"10".to_vec(),
        b"PARAMS".to_vec(),
        b"2".to_vec(),
        b"vec".to_vec(),
        vec![0, 0, 0, 63],
    ]
}

#[tokio::test]
async fn live_port_sends_ft_search_through_the_live_client() {
    let server = FakeValkey::start(REPLY.as_bytes().to_vec()).await;
    let connector = LiveValkeyConnector {
        factory: LiveClientFactory::new(None),
    };
    let url = format!("redis://{}:{}", server.addr.ip(), server.addr.port());
    let port = connector
        .connect(
            &url,
            SocketTimeouts {
                connect: Duration::from_secs(5),
                read: Duration::from_secs(5),
            },
        )
        .await
        .expect("the live port");
    let documents = port
        .search("vs_index", &query(), &search_params())
        .await
        .expect("the documents");
    assert_eq!(
        documents,
        vec![document(
            "doc-a",
            &[("text", "first"), ("vector_distance", "0.25")]
        )]
    );
    let commands = server.commands();
    let search = commands
        .iter()
        .find(|args| {
            args.first()
                .is_some_and(|name| name.as_slice() == b"FT.SEARCH")
        })
        .expect("the FT.SEARCH command");
    assert_eq!(search, &expected_search_arguments());
}

#[test]
fn wire_reads_a_resp2_array_reply() {
    let reply = Value::Array(vec![
        Value::Int(1),
        Value::BulkString(b"doc-a".to_vec()),
        Value::Array(vec![
            Value::BulkString(b"text".to_vec()),
            Value::BulkString(b"first".to_vec()),
            Value::BulkString(b"vector_distance".to_vec()),
            Value::BulkString(b"0.25".to_vec()),
        ]),
    ]);
    assert_eq!(
        wire::documents(&reply).expect("the documents"),
        vec![document(
            "doc-a",
            &[("text", "first"), ("vector_distance", "0.25")]
        )]
    );
}

#[test]
fn wire_reads_a_resp3_map_reply() {
    let reply = Value::Map(vec![
        (Value::BulkString(b"total_results".to_vec()), Value::Int(1)),
        (
            Value::BulkString(b"results".to_vec()),
            Value::Array(vec![Value::Map(vec![
                (
                    Value::BulkString(b"id".to_vec()),
                    Value::BulkString(b"doc-a".to_vec()),
                ),
                (
                    Value::BulkString(b"extra_attributes".to_vec()),
                    Value::Map(vec![
                        (
                            Value::BulkString(b"text".to_vec()),
                            Value::BulkString(b"first".to_vec()),
                        ),
                        (
                            Value::BulkString(b"vector_distance".to_vec()),
                            Value::BulkString(b"0.25".to_vec()),
                        ),
                    ]),
                ),
            ])]),
        ),
    ]);
    assert_eq!(
        wire::documents(&reply).expect("the documents"),
        vec![document(
            "doc-a",
            &[("text", "first"), ("vector_distance", "0.25")]
        )]
    );
}

#[test]
fn wire_reads_an_empty_reply_as_no_documents() {
    let reply = Value::Array(vec![Value::Int(0)]);
    assert_eq!(wire::documents(&reply).expect("the documents"), Vec::new());
}

#[rstest]
#[case::scalar(Value::Int(3))]
#[case::id_without_fields(Value::Array(vec![
    Value::Int(1),
    Value::BulkString(b"doc".to_vec()),
]))]
#[case::fields_not_an_array(Value::Array(vec![
    Value::Int(1),
    Value::BulkString(b"doc".to_vec()),
    Value::Int(2),
]))]
#[case::non_utf8_id(Value::Array(vec![
    Value::Int(1),
    Value::BulkString(vec![0xff]),
    Value::Array(Vec::new()),
]))]
#[case::map_result_without_id(Value::Map(vec![(
    Value::BulkString(b"results".to_vec()),
    Value::Array(vec![Value::Map(Vec::new())]),
)]))]
fn wire_rejects_malformed_replies(#[case] reply: Value) {
    let error = wire::documents(&reply).expect_err("the malformed reply");
    assert!(matches!(error, ValkeyError::UnexpectedReply { .. }));
}
