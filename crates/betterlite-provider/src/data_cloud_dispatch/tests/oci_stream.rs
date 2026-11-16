use std::time::Duration;

use betterlite_http::error::HttpError;
use betterlite_prompts::arguments::json::{Json as PromptJson, Object};
use futures_util::stream::{self, StreamExt as _};
use http::HeaderMap;

use crate::data_cloud_dispatch::error::DispatchError;
use crate::data_cloud_dispatch::oci_stream::{OciStreamRequest, open_stream};
use crate::data_cloud_dispatch::transport::StreamHead;
use crate::oci_chat::stream::StreamItem;

use super::support::Scripted;

fn sse_head(events: Vec<&'static str>) -> StreamHead {
    let chunks = events
        .into_iter()
        .map(|event| Ok(event.as_bytes().to_vec()))
        .collect::<Vec<Result<Vec<u8>, HttpError>>>();
    StreamHead {
        status: 200,
        body: Vec::new(),
        chunks: Some(stream::iter(chunks).boxed()),
    }
}

#[tokio::test]
async fn oci_stream_sends_the_signed_body_and_decodes_the_events() {
    let transport = Scripted::with_stream(sse_head(vec![
        "data: {\"message\":{\"content\":[{\"type\":\"TEXT\",\"text\":\"he\"}]}}\n",
        "\ndata: {\"message\":{\"content\":[{\"type\":\"TEXT\",\"text\":\"llo\"}]}}\n\ndata: [DONE]\n",
    ]));
    let headers = HeaderMap::new();
    let data = PromptJson::Object(Object::from_iter([(
        String::from("model"),
        PromptJson::Text(String::from("cohere.command-r-plus")),
    )]));
    let mut stream = open_stream(
        OciStreamRequest {
            api_base: "https://inference.generativeai.us-ashburn-1.oci.oraclecloud.com/20231130/actions/chat",
            headers,
            data: &data,
            signed_body: Some(b"{\"signed\":true}".to_vec()),
        },
        &transport,
    )
    .await
    .expect("the stream opens");
    let mut contents = Vec::new();
    let mut saw_done = false;
    while let Some(item) = stream.next().await.expect("the stream decodes") {
        match item {
            StreamItem::Done => {
                saw_done = true;
                break;
            }
            StreamItem::Chunk(chunk) => {
                if let Some(content) = chunk.choice.delta.content {
                    contents.push(content);
                }
            }
        }
    }
    assert_eq!(contents, vec![String::from("he"), String::from("llo")]);
    assert!(saw_done);
    assert!(stream.next().await.expect("the stream ends").is_none());
    let sent = transport.sent();
    assert_eq!(sent[0].method, "POST");
    assert_eq!(sent[0].body.as_slice(), b"{\"signed\":true}");
    assert_eq!(sent[0].deadline, Duration::from_secs(300));
}

#[tokio::test]
async fn oci_stream_sends_the_json_body_without_a_signature() {
    let transport = Scripted::with_stream(sse_head(vec!["data: [DONE]\n"]));
    let headers = HeaderMap::new();
    let data = PromptJson::Object(Object::from_iter([(
        String::from("model"),
        PromptJson::Text(String::from("cohere.command-r-plus")),
    )]));
    let stream = open_stream(
        OciStreamRequest {
            api_base: "https://inference.generativeai.us-ashburn-1.oci.oraclecloud.com/20231130/actions/chat",
            headers,
            data: &data,
            signed_body: None,
        },
        &transport,
    )
    .await;
    assert!(stream.is_ok());
    let sent = transport.sent();
    let body: PromptJson = serde_json::from_slice(&sent[0].body).expect("the body is JSON");
    assert_eq!(
        body,
        PromptJson::Object(Object::from_iter([(
            String::from("model"),
            PromptJson::Text(String::from("cohere.command-r-plus")),
        )]))
    );
}

#[tokio::test]
async fn oci_stream_maps_a_status_failure_to_the_oci_error() {
    let transport = Scripted::with_stream(StreamHead {
        status: 500,
        body: b"boom".to_vec(),
        chunks: None,
    });
    let headers = HeaderMap::new();
    let data = PromptJson::Null;
    match open_stream(
        OciStreamRequest {
            api_base: "https://inference.generativeai.us-ashburn-1.oci.oraclecloud.com/20231130/actions/chat",
            headers,
            data: &data,
            signed_body: None,
        },
        &transport,
    )
    .await
    {
        Err(DispatchError::Oci(error)) => {
            assert_eq!(error.status, 500);
            assert_eq!(error.message, "boom");
        }
        other => panic!("expected the OCI status error, got {other:?}"),
    }
}

#[tokio::test]
async fn oci_stream_carries_a_utf8_sequence_split_across_reads() {
    let line = "data: {\"message\":{\"content\":[{\"type\":\"TEXT\",\"text\":\"héllo\"}]}}\n\n";
    let bytes = line.as_bytes();
    let split = line
        .find('é')
        .expect("the fixture holds a two-byte sequence")
        + 1;
    let first = Ok(bytes[..split].to_vec());
    let second = Ok(bytes[split..].to_vec());
    let transport = Scripted::with_stream(StreamHead {
        status: 200,
        body: Vec::new(),
        chunks: Some(stream::iter(vec![first, second]).boxed()),
    });
    let headers = HeaderMap::new();
    let data = PromptJson::Null;
    let mut stream = open_stream(
        OciStreamRequest {
            api_base: "https://example.com",
            headers,
            data: &data,
            signed_body: None,
        },
        &transport,
    )
    .await
    .expect("the stream opens");
    let mut contents = Vec::new();
    while let Some(item) = stream.next().await.expect("the stream decodes") {
        if let StreamItem::Chunk(chunk) = item
            && let Some(content) = chunk.choice.delta.content
        {
            contents.push(content);
        }
    }
    assert_eq!(contents, vec![String::from("héllo")]);
}
