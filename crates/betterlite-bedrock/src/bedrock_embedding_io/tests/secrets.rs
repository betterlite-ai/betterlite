use super::fixture::{
    ACCESS_KEY, Factory, FixtureClient, Reply, SECRET_KEY, SESSION_TOKEN, credentials, headers,
    now, object, runtime,
};
use crate::bedrock_embedding_io::signed_get::sign_get_request;
use crate::bedrock_embedding_io::transport::{
    EmbeddingCall, EmbeddingRequest, SuppliedClient, post_embedding,
};

const URL: &str =
    "https://bedrock-runtime.us-west-2.amazonaws.com/model/amazon.titan-embed-text-v1/invoke";

#[test]
fn bedrock_embedding_io_secret_safe_failure() {
    let signed = sign_get_request(
        &credentials(ACCESS_KEY, SECRET_KEY, Some(SESSION_TOKEN)),
        URL,
        &headers(&[("Content-Type", "application/json")]),
        "us-west-2",
        now(),
    )
    .expect("the fixture request signs");
    let data = object(r#"{"inputText":"hello"}"#);
    let factory = Factory::new(Reply::status(401, &[], "denied"));
    let call = EmbeddingCall {
        asynchronous: false,
        url: URL,
        headers: &signed.headers,
        data: &data,
        timeout: None,
    };
    let error = runtime()
        .block_on(post_embedding(
            &call,
            None::<SuppliedClient<'_, FixtureClient>>,
            &factory,
        ))
        .expect_err("the fixture status fails");
    let recorded = factory.requests();
    assert_eq!(recorded.len(), 1);
    let sent: &EmbeddingRequest = &recorded[0];
    assert!(sent.headers.iter().any(|(name, value)| {
        name.as_str() == "X-Amz-Security-Token" && value.as_str() == SESSION_TOKEN
    }));
    assert!(
        sent.headers
            .iter()
            .any(|(name, value)| name.as_str() == "Authorization" && value.contains(ACCESS_KEY)),
        "the signed request really carries the credential"
    );
    let rendered = [
        format!("{sent:?}"),
        format!(
            "{:?}",
            credentials(ACCESS_KEY, SECRET_KEY, Some(SESSION_TOKEN))
        ),
        format!("{error:?}"),
        error.to_string(),
    ];
    assert!(
        rendered.iter().all(|text| {
            [ACCESS_KEY, SECRET_KEY, SESSION_TOKEN]
                .iter()
                .all(|value| !text.contains(value))
        }),
        "a rendering names a fixture secret"
    );
    assert!(
        rendered[0].contains("Authorization"),
        "the request Debug still names the signed headers"
    );
}
