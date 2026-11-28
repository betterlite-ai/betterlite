use std::collections::VecDeque;
use std::io;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_bedrock::count_call::handler::CountTokensTransport;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_openai::api::audio_file_input::AuthorizedAudioFiles;
use betterlite_secret_managers::secret_lookup::ports::{AzureOidcTokens, OidcClock};
use http::{HeaderMap, Method};
use url::Url;

use crate::bedrock_other_dispatch::test_support::client::{FixtureClient, Replies, Reply};
use crate::bedrock_other_dispatch::test_support::record::Seen;
use crate::bedrock_other_dispatch::test_support::{Files, secret_lookup};

fn outbound() -> OutboundRequest {
    OutboundRequest {
        method: Method::POST,
        url: Url::parse(
            "https://bedrock-runtime.us-east-1.amazonaws.com/model/amazon.titan-embed-text-v1/invoke",
        )
        .expect("the fixture url parses"),
        query: Vec::new(),
        headers: HeaderMap::new(),
        body: RequestBody::Empty,
        deadline: Duration::from_secs(30),
    }
}

// A scripted reply's headers reach the response the family's leaf decodes
#[tokio::test]
async fn fixture_reply_headers_reach_the_transport() {
    let client = FixtureClient {
        replies: Replies(Arc::new(Mutex::new(VecDeque::from([Reply::with_headers(
            200,
            &[("x-amzn-requestid", "req-1")],
            "{}",
        )])))),
        seen: Seen::default(),
    };
    let response = CountTokensTransport::send(&client, outbound())
        .await
        .expect("the fixture answers the request");
    assert_eq!(
        response
            .headers
            .get("x-amzn-requestid")
            .and_then(|value| value.to_str().ok()),
        Some("req-1"),
        "the scripted reply header reaches the transport response"
    );
}

// The fixture's file and OIDC ports refuse every read, so a composition row never touches a real file or token
#[test]
fn fixture_ports_refuse_external_reads() {
    let path = Path::new("clip.mp3");
    let read_error = Files.read(path).expect_err("the fixture reads no file");
    assert_eq!(read_error.kind(), io::ErrorKind::NotFound);
    let open_error = Files.open(path).expect_err("the fixture opens no file");
    assert_eq!(open_error.kind(), io::ErrorKind::NotFound);
    let lookup = secret_lookup();
    let token = lookup
        .azure_tokens
        .token("scope")
        .expect("the fixture answers no azure token");
    assert!(token.is_none(), "the fixture hands out no azure token");
    assert_eq!(
        lookup.clock.unix_seconds(),
        0,
        "the fixture's OIDC clock stays at the epoch"
    );
}
