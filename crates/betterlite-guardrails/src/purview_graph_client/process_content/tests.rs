#![cfg(test)]

use rstest::rstest;

use crate::purview_graph_client::process_content::{ContentRequest, process_content};
use crate::purview_graph_client::test_support::{guardrail, standard_reply, start};
use crate::purview_graph_client::wire::Activity;

#[tokio::test]
async fn process_content_posts_the_entry_for_the_encoded_user() {
    let mock = start(standard_reply).await;
    let base = guardrail(&mock.base);
    let request = ContentRequest {
        user_id: "user@contoso.com",
        text: "card 4111",
        activity: Activity::DownloadText,
        etag: "\"etag-1\"",
        correlation_id: Some("thread-9"),
    };

    process_content(&base, &request).await.unwrap();

    let requests = mock.requests.lock().unwrap().clone();
    let sent = requests.last().unwrap();
    assert_eq!(
        sent.path,
        "/users/user%40contoso.com/dataSecurityAndGovernance/processContent"
    );
    assert_eq!(sent.headers.get("authorization").unwrap(), "Bearer tok");
    assert_eq!(sent.headers.get("if-none-match").unwrap(), "\"etag-1\"");
    assert_eq!(
        sent.body,
        r#"{"contentToProcess":{"contentEntries":[{"@odata.type":"microsoft.graph.processConversationMetadata","identifier":"00000000-0000-0000-0000-000000000000","content":{"@odata.type":"microsoft.graph.textContent","data":"card 4111"},"name":"LiteLLM message","correlationId":"thread-9","sequenceNumber":0,"isTruncated":false}],"activityMetadata":{"activity":"downloadText"},"deviceMetadata":{},"protectedAppMetadata":{"name":"LiteLLM","version":"1.0","applicationLocation":{"@odata.type":"microsoft.graph.policyLocationApplication","value":"client-1"}},"integratedAppMetadata":{"name":"LiteLLM","version":"1.0"}}}"#
    );
}

#[rstest]
#[case::with_etag("\"etag-1\"", true)]
#[case::without_etag("", false)]
#[tokio::test]
async fn process_content_sends_if_none_match_only_for_a_known_etag(
    #[case] etag: &str,
    #[case] sent: bool,
) {
    let mock = start(standard_reply).await;
    let base = guardrail(&mock.base);
    let request = ContentRequest {
        user_id: "user-1",
        text: "hello",
        activity: Activity::UploadText,
        etag,
        correlation_id: None,
    };

    process_content(&base, &request).await.unwrap();

    let requests = mock.requests.lock().unwrap().clone();
    let headers = &requests.last().unwrap().headers;
    assert_eq!(headers.contains_key("if-none-match"), sent);
}

#[tokio::test]
async fn process_content_generates_a_correlation_id_when_none_is_given() {
    let mock = start(standard_reply).await;
    let base = guardrail(&mock.base);
    let request = ContentRequest {
        user_id: "user-1",
        text: "hello",
        activity: Activity::UploadText,
        etag: "",
        correlation_id: None,
    };

    process_content(&base, &request).await.unwrap();

    let requests = mock.requests.lock().unwrap().clone();
    assert!(
        requests
            .last()
            .unwrap()
            .body
            .contains(r#""correlationId":"00000000-0000-0000-0000-000000000000""#)
    );
}
