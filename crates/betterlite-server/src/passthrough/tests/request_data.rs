use betterlite_core::api::api_boundaries::ApiLimits;

use crate::passthrough::request_data::{
    ParsedRequestData, ParsedRequestError, parse_request_by_content_type,
};

fn parse(
    content_type: &str,
    body: &[u8],
    query: &[(String, String)],
) -> Result<ParsedRequestData, ParsedRequestError> {
    parse_request_by_content_type(content_type, body, query, &ApiLimits::default())
}

fn query(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

mod passthrough_parse_content_type {
    use betterlite_core::api::api_boundaries::ApiLimits;
    use rstest::rstest;
    use serde_json::json;

    use super::{parse, query};
    use crate::passthrough::request_data::{
        ParsedRequestData, ParsedRequestError, parse_request_by_content_type,
    };

    #[test]
    fn a_json_envelope_is_read() {
        let parsed = parse(
            "application/json",
            br#"{"query_params":{"page":2},"custom_body":{"model":"m"},"stream":true}"#,
            &[],
        )
        .unwrap();
        assert_eq!(
            parsed,
            ParsedRequestData {
                query_params: Some(json!({"page": 2})),
                custom_body: Some(json!({"model": "m"})),
                stream: Some(true),
            }
        );
    }

    #[test]
    fn a_json_body_without_custom_body_keeps_it_absent() {
        let parsed = parse(
            "application/json",
            br#"{"model":"m","stream":false}"#,
            &query(&[("a", "1")]),
        )
        .unwrap();
        assert_eq!(
            parsed,
            ParsedRequestData {
                query_params: None,
                custom_body: None,
                stream: Some(false),
            }
        );
    }

    #[test]
    fn null_envelope_fields_take_their_defaults() {
        let parsed = parse(
            "application/json",
            br#"{"query_params":null,"custom_body":null,"stream":null}"#,
            &[],
        )
        .unwrap();
        assert_eq!(parsed, ParsedRequestData::default());
    }

    #[rstest]
    #[case::unterminated(b"{")]
    #[case::empty(b"")]
    #[case::garbage(b"not json")]
    fn a_body_that_is_not_json_is_no_envelope(#[case] body: &[u8]) {
        let parsed = parse("application/json", body, &query(&[("a", "1")])).unwrap();
        assert_eq!(parsed, ParsedRequestData::default());
    }

    #[rstest]
    #[case::array(b"[1,2]")]
    #[case::null(b"null")]
    #[case::string(b"\"text\"")]
    #[case::number(b"3")]
    fn a_non_object_json_body_is_rejected(#[case] body: &[u8]) {
        let error = parse("application/json", body, &[]).unwrap_err();
        assert!(matches!(error, ParsedRequestError::BodyNotObject));
    }

    #[rstest]
    #[case::stream(br#"{"stream":"yes"}"#, "stream")]
    #[case::query_params(br#"{"query_params":[]}"#, "query_params")]
    #[case::custom_body(br#"{"custom_body":"x"}"#, "custom_body")]
    fn a_wrongly_typed_envelope_field_is_rejected(
        #[case] body: &[u8],
        #[case] expected: &'static str,
    ) {
        match parse("application/json", body, &[]).unwrap_err() {
            ParsedRequestError::FieldType { field } => assert_eq!(field, expected),
            other => panic!("expected a field type error, got {other:?}"),
        }
    }

    #[test]
    fn a_multipart_json_body_registers_as_the_whole_custom_body() {
        let parsed = parse(
            "multipart/form-data; boundary=x",
            br#"{"model":"m","stream":true}"#,
            &[],
        )
        .unwrap();
        assert_eq!(
            parsed,
            ParsedRequestData {
                query_params: None,
                custom_body: Some(json!({"model": "m", "stream": true})),
                stream: Some(true),
            }
        );
    }

    #[test]
    fn a_multipart_json_body_with_custom_body_keeps_only_that() {
        let parsed = parse(
            "multipart/form-data; boundary=x",
            br#"{"custom_body":{"a":1}}"#,
            &[],
        )
        .unwrap();
        assert_eq!(
            parsed,
            ParsedRequestData {
                query_params: None,
                custom_body: Some(json!({"a": 1})),
                stream: None,
            }
        );
    }

    #[test]
    fn an_empty_json_object_under_multipart_has_no_custom_body() {
        let parsed = parse("multipart/form-data; boundary=x", b"{}", &[]).unwrap();
        assert_eq!(parsed, ParsedRequestData::default());
    }

    #[rstest]
    #[case::multipart_bytes(
        b"--x\r\nContent-Disposition: form-data; name=\"a\"\r\n\r\n1\r\n--x--\r\n"
    )]
    #[case::non_object_json(b"[1]")]
    fn a_multipart_body_that_is_not_an_envelope_is_left_alone(#[case] body: &[u8]) {
        let parsed = parse("multipart/form-data; boundary=x", body, &[]).unwrap();
        assert_eq!(parsed, ParsedRequestData::default());
    }

    #[test]
    fn urlencoded_fields_are_read() {
        let parsed = parse(
            "application/x-www-form-urlencoded",
            b"query_params=page%3D2&custom_body=%7B%22a%22%3A1%7D",
            &[],
        )
        .unwrap();
        assert_eq!(
            parsed,
            ParsedRequestData {
                query_params: Some(json!("page=2")),
                custom_body: Some(json!("{\"a\":1}")),
                stream: None,
            }
        );
    }

    #[test]
    fn urlencoded_repeated_names_keep_the_last_value() {
        let parsed = parse(
            "application/x-www-form-urlencoded",
            b"query_params=first&query_params=second",
            &query(&[("a", "1")]),
        )
        .unwrap();
        assert_eq!(parsed.query_params, Some(json!("second")));
        assert_eq!(parsed.custom_body, None);
    }

    #[test]
    fn urlencoded_with_another_media_type_yields_nothing() {
        let parsed = parse(
            "text/plain; application/x-www-form-urlencoded",
            b"query_params=x",
            &query(&[("a", "1")]),
        )
        .unwrap();
        assert_eq!(parsed, ParsedRequestData::default());
    }

    #[test]
    fn a_urlencoded_body_over_the_limit_is_an_error() {
        let limits = ApiLimits {
            max_json_bytes: 4,
            max_event_bytes: 1024,
            max_items: 64,
        };
        let error = parse_request_by_content_type(
            "application/x-www-form-urlencoded",
            b"a=1&b=2",
            &[],
            &limits,
        )
        .unwrap_err();
        assert!(matches!(error, ParsedRequestError::Form(_)));
    }

    #[test]
    fn an_unrecognized_content_type_falls_back_to_the_query() {
        let parsed = parse(
            "text/plain",
            b"",
            &query(&[("a", "1"), ("a", "2"), ("b", "3")]),
        )
        .unwrap();
        assert_eq!(
            parsed,
            ParsedRequestData {
                query_params: Some(json!({"a": "2", "b": "3"})),
                custom_body: None,
                stream: None,
            }
        );
    }

    #[rstest]
    #[case::no_content_type_default("", b"")]
    #[case::no_content_type_body("", b"ignored")]
    #[case::other("application/octet-stream", b"ignored")]
    fn an_unrecognized_content_type_without_a_query_has_nothing(
        #[case] content_type: &str,
        #[case] body: &[u8],
    ) {
        let parsed = parse(content_type, body, &[]).unwrap();
        assert_eq!(parsed, ParsedRequestData::default());
    }

    #[test]
    fn the_json_branch_wins_over_the_multipart_branch() {
        let error = parse(
            "multipart/form-data; application/json",
            b"[1]",
            &query(&[("a", "1")]),
        )
        .unwrap_err();
        assert!(matches!(error, ParsedRequestError::BodyNotObject));
    }

    #[test]
    fn the_json_branch_wins_over_the_urlencoded_branch() {
        let parsed = parse(
            "application/json; application/x-www-form-urlencoded",
            br#"{"stream":true}"#,
            &[],
        )
        .unwrap();
        assert_eq!(parsed.stream, Some(true));
    }
}
