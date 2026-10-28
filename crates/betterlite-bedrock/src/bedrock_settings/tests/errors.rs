use betterlite_openai::error::{ErrorRequest, ErrorResponse};

use crate::bedrock_settings::{
    BedrockError, ErrorHeaders, ErrorInput, ErrorReply, HeaderError, HeaderValue, ResponseBody,
    error_response_text,
};

const DOCS: &str = "https://docs.litellm.ai/docs";

pub(super) fn text(value: &str) -> HeaderValue {
    HeaderValue::Text(value.to_owned())
}

fn pairs(entries: &[(&str, &str)]) -> Vec<(String, String)> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn mapping(entries: Vec<(&str, HeaderValue)>) -> ErrorHeaders {
    let named = entries
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value));
    ErrorHeaders::Mapping(named.collect())
}

fn request(url: &str) -> ErrorRequest {
    ErrorRequest {
        method: "GET".to_owned(),
        url: url.to_owned(),
    }
}

fn reply(headers: &[(&str, &str)], body: ResponseBody) -> ErrorReply {
    let response = ErrorResponse {
        status_code: 500,
        request: request("https://reply.example"),
    };
    ErrorReply {
        response,
        headers: pairs(headers),
        body,
    }
}

pub(super) fn input(headers: Option<ErrorHeaders>, response: Option<ErrorReply>) -> ErrorInput {
    ErrorInput {
        status_code: 413,
        message: "too large".to_owned(),
        request: None,
        body: Some(serde_json::from_str(r#"{"Message": "too large", "count": 7}"#).unwrap()),
        status_code_is_synthesized: false,
        headers,
        response,
    }
}

fn build(headers: Option<ErrorHeaders>, response: Option<ErrorReply>) -> BedrockError {
    BedrockError::try_from(input(headers, response)).unwrap()
}

fn synthesizes_safe_mapping_headers() {
    let headers = Some(mapping(vec![
        ("X-A", text("1")),
        ("x-b", HeaderValue::Bytes(vec![0xc3, 0xa9])),
        ("x-count", HeaderValue::Integer(7)),
        ("x-none", HeaderValue::Null),
        ("x-flag", HeaderValue::Boolean(true)),
    ]));
    let error = build(headers.clone(), None);
    assert_eq!(error.headers, headers);
    assert_eq!(error.error.headers, Some(pairs(&[("X-A", "1")])));
    assert_eq!(
        error.response_headers,
        pairs(&[("X-A", "1"), ("x-b", "\u{e9}")])
    );
    let docs = ErrorRequest {
        method: "POST".to_owned(),
        url: DOCS.to_owned(),
    };
    assert_eq!(
        (&error.error.request, &error.error.response.request),
        (&docs, &docs)
    );
    assert_eq!(error.error.response.status_code, 413);
    assert!(!error.error.status_code_is_synthesized);
    assert_eq!(error.error.message, "too large");
    assert_eq!(error.error.body, input(None, None).body);
    let latin_headers = mapping(vec![
        ("x", HeaderValue::Bytes(vec![0xff])),
        ("y", HeaderValue::Bytes(vec![0xc3, 0xa9])),
    ]);
    let latin = build(Some(latin_headers), None);
    assert_eq!(
        latin.response_headers,
        pairs(&[("x", "\u{ff}"), ("y", "\u{c3}\u{a9}")])
    );
}

fn keeps_inputs_unchanged() {
    let count = Some(mapping(vec![("x-count", HeaderValue::Integer(7))]));
    let only_count = build(count.clone(), None);
    assert_eq!(only_count.headers, count);
    assert!(only_count.response_headers.is_empty());
    let http = pairs(&[("X-A", "1"), ("x-a", "2")]);
    let mut supplied = input(Some(ErrorHeaders::Http(http.clone())), None);
    supplied.request = Some(request("https://request.example"));
    let kept = BedrockError::try_from(supplied).unwrap();
    assert_eq!(
        (&kept.response_headers, kept.error.headers.as_ref()),
        (&http, Some(&http))
    );
    assert_eq!(
        kept.error.response.request,
        request("https://request.example")
    );
    let empty = build(Some(mapping(Vec::new())), None);
    assert_eq!(empty.error.headers, Some(Vec::new()));
    assert_eq!(empty.error.response, build(None, None).error.response);
}

fn existing_response_wins() {
    let existing = reply(
        &[("x-reply", "yes")],
        ResponseBody::Read("reply".to_owned()),
    );
    let not_ascii = Some(mapping(vec![("x", text("\u{e9}"))]));
    let error = build(not_ascii.clone(), Some(existing.clone()));
    assert_eq!(error.error.response, existing.response);
    assert_eq!(error.response_headers, existing.headers);
    assert_eq!(error.error.request.url, DOCS);
    let refused = BedrockError::try_from(input(not_ascii, None));
    assert_eq!(
        refused,
        Err(HeaderError::NotAscii {
            name: "x".to_owned()
        })
    );
}

pub(super) fn bedrock_error_response() {
    synthesizes_safe_mapping_headers();
    keeps_inputs_unchanged();
    existing_response_wins();
}

pub(super) fn error_text_unread() {
    let phrase = "Internal Server Error";
    let unread = ResponseBody::Unread {
        reason_phrase: phrase.to_owned(),
    };
    assert_eq!(error_response_text(&unread), phrase);
    assert_eq!(
        error_response_text(&ResponseBody::Read("{}".to_owned())),
        "{}"
    );
    let empty = ResponseBody::Unread {
        reason_phrase: String::new(),
    };
    assert_eq!(error_response_text(&empty), "");
    let count = Some(mapping(vec![("x-count", HeaderValue::Integer(7))]));
    let supplied = build(count.clone(), Some(reply(&[], unread.clone())));
    assert_eq!(supplied.response_body, unread);
    assert_eq!(error_response_text(&supplied.response_body), phrase);
    assert_eq!(error_response_text(&build(count, None).response_body), "");
}
