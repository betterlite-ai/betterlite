use betterlite_prompts::arguments::json::Json;

use crate::openapi_tools::tool::{OpenApiTool, OpenApiToolError, OpenApiToolOutput};

use super::support::{FakeTransport, arguments, config, context, json, tool};

async fn reply(status: u16, body: &[u8]) -> Result<OpenApiToolOutput, OpenApiToolError> {
    let tool = tool("get", "/pets", "{}");
    let transport = FakeTransport::replying(status, body);
    tool.call(&arguments("{}"), &context(), &transport).await
}

#[tokio::test]
async fn a_json_object_body_is_a_json_result() {
    assert_eq!(
        reply(200, br#"{"id":1}"#).await.unwrap(),
        OpenApiToolOutput::Json {
            value: json(r#"{"id":1}"#),
            original_text: r#"{"id":1}"#.to_owned(),
        }
    );
}

#[tokio::test]
async fn a_json_array_body_is_a_json_result() {
    assert_eq!(
        reply(200, b"[1,2]").await.unwrap(),
        OpenApiToolOutput::Json {
            value: json("[1,2]"),
            original_text: "[1,2]".to_owned(),
        }
    );
}

#[tokio::test]
async fn a_json_scalar_body_is_a_json_result() {
    assert_eq!(
        reply(200, b"true").await.unwrap(),
        OpenApiToolOutput::Json {
            value: Json::Bool(true),
            original_text: "true".to_owned(),
        }
    );
}

#[tokio::test]
async fn a_json_null_body_stays_text() {
    assert_eq!(
        reply(200, b"null").await.unwrap(),
        OpenApiToolOutput::Text("null".to_owned())
    );
}

#[tokio::test]
async fn an_invalid_json_body_stays_text() {
    assert_eq!(
        reply(200, b"not json").await.unwrap(),
        OpenApiToolOutput::Text("not json".to_owned())
    );
}

#[tokio::test]
async fn an_empty_body_stays_text() {
    let output = reply(204, b"").await.unwrap();
    assert_eq!(output, OpenApiToolOutput::Text(String::new()));
}

#[tokio::test]
async fn a_whitespace_body_stays_text() {
    let output = reply(200, b"  \n").await.unwrap();
    assert_eq!(output, OpenApiToolOutput::Text("  \n".to_owned()));
}

#[tokio::test]
async fn a_status_below_400_is_parsed_as_output() {
    assert_eq!(
        reply(302, b"{}").await.unwrap(),
        OpenApiToolOutput::Json {
            value: json("{}"),
            original_text: "{}".to_owned(),
        }
    );
}

#[tokio::test]
async fn a_status_of_400_is_an_upstream_status_without_the_body() {
    let error = reply(400, b"sensitive upstream document")
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        OpenApiToolError::UpstreamStatus { status: 400, ref server } if server == "GET /pets"
    ));
    assert!(!error.to_string().contains("sensitive"));
}

#[tokio::test]
async fn a_status_of_500_is_an_upstream_status() {
    let error = reply(500, b"").await.unwrap_err();
    assert!(matches!(
        error,
        OpenApiToolError::UpstreamStatus { status: 500, .. }
    ));
}

#[tokio::test]
async fn a_403_is_an_upstream_status_even_when_relaying() {
    let mut config = config("get", "/pets", "{}");
    config.relays_upstream_auth = true;
    let tool = OpenApiTool::new(config);
    let transport = FakeTransport::replying(403, b"").with_header("www-authenticate", "Bearer");
    let error = tool
        .call(&arguments("{}"), &context(), &transport)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        OpenApiToolError::UpstreamStatus { status: 403, .. }
    ));
}

#[tokio::test]
async fn a_401_without_relaying_is_an_upstream_status() {
    let tool = tool("get", "/pets", "{}");
    let transport = FakeTransport::replying(401, b"").with_header("www-authenticate", "Bearer");
    let error = tool
        .call(&arguments("{}"), &context(), &transport)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        OpenApiToolError::UpstreamStatus { status: 401, .. }
    ));
}

#[tokio::test]
async fn a_relayed_401_carries_the_challenge() {
    let mut config = config("get", "/pets", "{}");
    config.relays_upstream_auth = true;
    let tool = OpenApiTool::new(config);
    let transport = FakeTransport::replying(401, b"denied")
        .with_header("www-authenticate", "Bearer realm=\"upstream\"");
    let error = tool
        .call(&arguments("{}"), &context(), &transport)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        OpenApiToolError::UpstreamAuth {
            status: 401,
            www_authenticate: Some(ref challenge),
            ref server,
        } if challenge == "Bearer realm=\"upstream\"" && server == "GET /pets"
    ));
}

#[tokio::test]
async fn a_relayed_401_without_a_challenge_has_none() {
    let mut config = config("get", "/pets", "{}");
    config.relays_upstream_auth = true;
    let tool = OpenApiTool::new(config);
    let transport = FakeTransport::replying(401, b"");
    let error = tool
        .call(&arguments("{}"), &context(), &transport)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        OpenApiToolError::UpstreamAuth {
            www_authenticate: None,
            ..
        }
    ));
}

#[tokio::test]
async fn repeated_challenge_headers_are_joined() {
    let mut config = config("get", "/pets", "{}");
    config.relays_upstream_auth = true;
    config.server_label = Some("petstore".to_owned());
    let tool = OpenApiTool::new(config);
    let transport = FakeTransport::replying(401, b"")
        .with_header("www-authenticate", "Bearer a")
        .with_header("www-authenticate", "Basic b");
    let error = tool
        .call(&arguments("{}"), &context(), &transport)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        OpenApiToolError::UpstreamAuth {
            www_authenticate: Some(ref challenge),
            ref server,
            ..
        } if challenge == "Bearer a, Basic b" && server == "petstore"
    ));
}

#[tokio::test]
async fn a_latin_1_challenge_is_decoded_byte_by_byte() {
    let mut config = config("get", "/pets", "{}");
    config.relays_upstream_auth = true;
    let tool = OpenApiTool::new(config);
    let transport =
        FakeTransport::replying(401, b"").with_header_bytes("www-authenticate", b"Basic a\xe9b");
    let error = tool
        .call(&arguments("{}"), &context(), &transport)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        OpenApiToolError::UpstreamAuth {
            www_authenticate: Some(ref challenge),
            ..
        } if challenge == "Basic a\u{e9}b"
    ));
}

#[tokio::test]
async fn a_utf8_challenge_is_decoded_as_utf8() {
    let mut config = config("get", "/pets", "{}");
    config.relays_upstream_auth = true;
    let tool = OpenApiTool::new(config);
    let transport = FakeTransport::replying(401, b"")
        .with_header_bytes("www-authenticate", b"Basic caf\xc3\xa9");
    let error = tool
        .call(&arguments("{}"), &context(), &transport)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        OpenApiToolError::UpstreamAuth {
            www_authenticate: Some(ref challenge),
            ..
        } if challenge == "Basic café"
    ));
}

#[tokio::test]
async fn a_non_utf8_body_is_an_error() {
    let tool = tool("get", "/pets", "{}");
    let transport = FakeTransport::replying(200, &[0xFF, 0xFE]);
    let error = tool
        .call(&arguments("{}"), &context(), &transport)
        .await
        .unwrap_err();
    assert!(matches!(error, OpenApiToolError::BodyText { .. }));
}

#[tokio::test]
async fn a_utf8_body_with_multibyte_text_is_fine() {
    assert_eq!(
        reply(200, "café".as_bytes()).await.unwrap(),
        OpenApiToolOutput::Text("café".to_owned())
    );
}
