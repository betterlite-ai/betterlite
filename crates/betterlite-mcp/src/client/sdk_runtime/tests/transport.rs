use crate::client::sdk_transport::error::SdkError;
use crate::client::sessions::error::SessionError;
use crate::client::sessions::transport::TransportType;
use crate::client::tools::listing::list_tools;

use super::support;

#[tokio::test]
async fn an_unparseable_url_leaves_a_transport_that_cannot_open() {
    for (transport, expected) in [
        (TransportType::Http, "streamable HTTP"),
        (TransportType::Sse, "legacy SSE"),
    ] {
        let mut client = support::client_for(
            support::runtime(),
            support::settings(transport),
            "not a url",
            support::CALL_TIMEOUT,
        );
        let error = list_tools(&mut client, support::CALL_TIMEOUT, true)
            .await
            .expect_err("the transport cannot open");
        let SessionError::Runtime(source) = error else {
            panic!("expected a runtime failure");
        };
        let SdkError::Io(inner) = source.as_ref() else {
            panic!("expected the unavailable stdio transport");
        };
        assert_eq!(inner.kind(), std::io::ErrorKind::Unsupported);
        assert!(inner.to_string().contains(expected));
    }
}
