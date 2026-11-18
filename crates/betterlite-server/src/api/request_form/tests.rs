#![cfg(test)]

use std::collections::BTreeSet;

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::request_body::DecodedJson;
use betterlite_core::api::request_form::annotations::{FormAnnotation, numeric_form_fields};
use betterlite_core::api::request_form::coercion::coerce_numeric_form_fields;
use betterlite_core::api::request_form::convert::{
    ConvertError, FileDataValue, FileTuple, convert_uploads_to_file_data,
};
use betterlite_core::api::request_form::file_size::{
    ModelUploadLimits, UploadLimit, UploadSizeCheck, check_file_size_under_limit,
};
use betterlite_core::api::request_form::metadata::{
    MetadataValue, MetadataWarning, MetadataWarningKind, extract_nested_form_metadata,
};
use betterlite_core::api::request_form::values::{
    FormError, FormFields, FormValue, NonFinite, UploadFile, get_form_data,
};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use rstest::rstest;

use super::{RequestFormBody, RequestFormError, decode_request_form, get_request_body};

const URLENCODED: &[u8] = b"application/x-www-form-urlencoded";
const MULTIPART: &[u8] = b"multipart/form-data; boundary=b";

fn text_part(name: &str, value: &str) -> String {
    format!("--b\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n")
}

fn upload_part(name: &str, filename: &str, content: &str) -> String {
    format!(
        "--b\r\nContent-Disposition: form-data; name=\"{name}\"; filename=\"{filename}\"\r\nContent-Type: text/plain\r\n\r\n{content}\r\n"
    )
}

fn decode_multipart_parts(body: String) -> FormFields {
    let terminated = format!("{body}--b--\r\n");
    let parts =
        decode_request_form(terminated.as_bytes(), MULTIPART, &ApiLimits::default()).unwrap();
    get_form_data(&parts).unwrap()
}

fn upload_value(filename: &str, content: &str) -> FormValue {
    FormValue::Upload(UploadFile {
        filename: filename.to_owned(),
        content_type: Some("text/plain".to_owned()),
        content: content.as_bytes().to_vec(),
        size: Some(u64::try_from(content.len()).unwrap()),
    })
}

#[test]
fn repeated_plain_last_wins() {
    let parts =
        decode_request_form(b"model=first&model=last", URLENCODED, &ApiLimits::default()).unwrap();
    let fields = get_form_data(&parts).unwrap();
    assert_eq!(
        fields.get("model"),
        Some(&FormValue::Text("last".to_owned()))
    );

    let uploads = decode_multipart_parts(upload_part("a", "a.txt", "abc"));
    let data = convert_uploads_to_file_data(&uploads).unwrap();
    assert_eq!(
        data.get("a"),
        Some(&FileDataValue::Files(vec![FileTuple {
            filename: "a.txt".to_owned(),
            content: b"abc".to_vec(),
            content_type: Some("text/plain".to_owned()),
        }]))
    );
}

#[test]
fn a_mixed_list_is_decided_by_its_first_entry() {
    let fields = decode_multipart_parts(format!(
        "{}{}",
        text_part("files[]", "plain"),
        upload_part("files[]", "a.txt", "a")
    ));
    let data = convert_uploads_to_file_data(&fields).unwrap();
    assert_eq!(
        data.get("files"),
        Some(&FileDataValue::Unchanged(FormValue::List(vec![
            FormValue::Text("plain".to_owned()),
            upload_value("a.txt", "a"),
        ])))
    );

    let upload_first = decode_multipart_parts(format!(
        "{}{}",
        upload_part("files[]", "a.txt", "a"),
        text_part("files[]", "plain")
    ));
    assert_eq!(
        convert_uploads_to_file_data(&upload_first).unwrap_err(),
        ConvertError::MissingRead {
            name: "files".to_owned(),
            index: 1,
        }
    );
}

#[test]
fn array_form_preserves_order() {
    let parts = decode_request_form(
        b"timestamp_granularities%5B%5D=word&timestamp_granularities%5B%5D=segment",
        URLENCODED,
        &ApiLimits::default(),
    )
    .unwrap();
    let fields = get_form_data(&parts).unwrap();
    assert_eq!(
        fields.get("timestamp_granularities"),
        Some(&FormValue::List(vec![
            FormValue::Text("word".to_owned()),
            FormValue::Text("segment".to_owned()),
        ]))
    );

    let leaf_first =
        decode_request_form(b"x=leaf&x%5B%5D=child", URLENCODED, &ApiLimits::default()).unwrap();
    assert_eq!(
        get_form_data(&leaf_first).unwrap_err(),
        FormError::ScalarAppend {
            name: "x".to_owned(),
        }
    );

    let child_first =
        decode_request_form(b"x%5B%5D=child&x=leaf", URLENCODED, &ApiLimits::default()).unwrap();
    assert_eq!(
        get_form_data(&child_first).unwrap().get("x"),
        Some(&FormValue::Text("leaf".to_owned()))
    );
}

#[test]
fn numeric_form_coercion() {
    let annotations: betterlite_core::api::request_form::annotations::FormAnnotations = [
        ("count".to_owned(), FormAnnotation::Int),
        ("ratio".to_owned(), FormAnnotation::Float),
        ("nan_ratio".to_owned(), FormAnnotation::Float),
        (
            "optional".to_owned(),
            FormAnnotation::Union(vec![FormAnnotation::Int, FormAnnotation::NoneType]),
        ),
        ("flag".to_owned(), FormAnnotation::Other),
        (
            "pair".to_owned(),
            FormAnnotation::Union(vec![FormAnnotation::Int, FormAnnotation::Float]),
        ),
    ]
    .into_iter()
    .collect();
    let numeric = numeric_form_fields(&annotations);
    let parts = decode_request_form(
        b"count=%207%20&ratio=1e2&nan_ratio=nan&optional=%2B3&flag=true&pair=2",
        URLENCODED,
        &ApiLimits::default(),
    )
    .unwrap();
    let fields = get_form_data(&parts).unwrap();
    let coerced = coerce_numeric_form_fields(&fields, &numeric);
    assert_eq!(
        coerced.get("count"),
        Some(&FormValue::Number(serde_json::Number::from(7_i64)))
    );
    assert_eq!(
        coerced.get("ratio"),
        Some(&FormValue::Number(
            serde_json::Number::from_f64(100.0).unwrap()
        ))
    );
    assert_eq!(
        coerced.get("nan_ratio"),
        Some(&FormValue::NonFinite(NonFinite::Nan))
    );
    assert_eq!(
        coerced.get("optional"),
        Some(&FormValue::Number(serde_json::Number::from(3_i64)))
    );
    assert_eq!(
        coerced.get("flag"),
        Some(&FormValue::Text("true".to_owned()))
    );
    assert_eq!(coerced.get("pair"), Some(&FormValue::Text("2".to_owned())));
}

#[test]
fn nested_form_metadata() {
    let parts = decode_request_form(
        b"litellm_metadata%5Ba%5D%5Bb%5D=v&litellm_metadata%5Bteam%5D=alpha&other=x",
        URLENCODED,
        &ApiLimits::default(),
    )
    .unwrap();
    let fields = get_form_data(&parts).unwrap();
    let (metadata, warnings) = extract_nested_form_metadata(&fields, "litellm_metadata[");
    assert!(warnings.is_empty());
    let Some(MetadataValue::Object(child)) = metadata.get("a") else {
        panic!("expected a nested object");
    };
    assert_eq!(
        child.get("b"),
        Some(&MetadataValue::Leaf(FormValue::Text("v".to_owned())))
    );
    assert_eq!(
        metadata.get("team"),
        Some(&MetadataValue::Leaf(FormValue::Text("alpha".to_owned())))
    );
    assert!(!metadata.contains_key("other"));

    let conflicting = decode_multipart_parts(format!(
        "{}{}{}",
        text_part("litellm_metadata[a]", "leaf"),
        text_part("litellm_metadata[a][b]", "deep"),
        upload_part("litellm_metadata[file]", "a.txt", "abc")
    ));
    let (conflict_metadata, conflict_warnings) =
        extract_nested_form_metadata(&conflicting, "litellm_metadata[");
    assert_eq!(
        conflict_metadata.get("a"),
        Some(&MetadataValue::Leaf(FormValue::Text("leaf".to_owned())))
    );
    assert_eq!(
        conflict_warnings,
        vec![
            MetadataWarning {
                kind: MetadataWarningKind::ParentNotObject,
                subject: "litellm_metadata[a][b]".to_owned(),
            },
            MetadataWarning {
                kind: MetadataWarningKind::UploadSkipped,
                subject: "litellm_metadata[file]".to_owned(),
            },
        ]
    );
}

struct FixedRouter {
    limit: UploadLimit,
}

impl ModelUploadLimits for FixedRouter {
    fn upload_limit_mebibytes(&self, _model: &str) -> UploadLimit {
        self.limit
    }
}

fn limits_of(router: Option<&FixedRouter>) -> Option<&dyn ModelUploadLimits> {
    match router {
        Some(fixed) => Some(fixed),
        None => None,
    }
}

fn size_mb_of(data: &JsonObject) -> Option<f64> {
    match data.get("metadata") {
        Some(Json::Object(metadata)) => match metadata.get("file_size_in_mb") {
            Some(Json::Number(number)) => number.as_f64(),
            _ => None,
        },
        _ => None,
    }
}

#[rstest]
#[case::absent_size(None, 0.0)]
#[case::one_mebibyte(Some(0x0010_0000), 1.0)]
fn upload_size_unset(#[case] size: Option<u64>, #[case] expected_mb: f64) {
    let mut request_data = JsonObject::new();
    request_data.insert("model".to_owned(), Json::Text("whisper".to_owned()));
    let mut metadata = JsonObject::new();
    metadata.insert("team".to_owned(), Json::Text("t".to_owned()));
    request_data.insert("metadata".to_owned(), Json::Object(metadata));
    let outcome = check_file_size_under_limit(UploadSizeCheck {
        request_data: &mut request_data,
        file_size: size,
        router_names: &BTreeSet::new(),
        router: None,
        limits: &ApiLimits::default(),
    })
    .unwrap();
    assert!(outcome.proceeded);
    assert_eq!(size_mb_of(&request_data), Some(expected_mb));
    let Some(Json::Object(recorded_metadata)) = request_data.get("metadata") else {
        panic!("metadata should stay an object");
    };
    assert_eq!(
        recorded_metadata.get("team"),
        Some(&Json::Text("t".to_owned()))
    );
}

#[rstest]
#[case::router_absent(&[], UploadLimit::Configured(1.0), 0x0020_0000, false)]
#[case::model_outside_names(&["other"], UploadLimit::Configured(1.0), 0x0020_0000, false)]
#[case::deployment_absent(&["whisper"], UploadLimit::Absent, 0x0020_0000, false)]
#[case::lookup_failed(&["whisper"], UploadLimit::Failed, 0x0020_0000, true)]
fn upload_size_skips_a_missing_model_limit(
    #[case] router_names: &[&str],
    #[case] limit: UploadLimit,
    #[case] size: u64,
    #[case] lookup_failed: bool,
) {
    let router = FixedRouter { limit };
    let names: BTreeSet<String> = router_names.iter().map(|name| (*name).to_owned()).collect();
    let mut request_data = JsonObject::new();
    request_data.insert("model".to_owned(), Json::Text("whisper".to_owned()));
    let outcome = check_file_size_under_limit(UploadSizeCheck {
        request_data: &mut request_data,
        file_size: Some(size),
        router_names: &names,
        router: Some(&router),
        limits: &ApiLimits::default(),
    })
    .unwrap();
    assert!(outcome.proceeded);
    assert_eq!(outcome.lookup_failed, lookup_failed);
}

#[test]
fn request_body_dispatch() {
    use betterlite_config::sections::request_body::Settings;
    use betterlite_core::api::request_body::RequestBodyInput;
    use std::collections::BTreeMap;

    let limits = ApiLimits::default();
    assert_eq!(
        get_request_body("GET", "application/json", b"garbage", &limits),
        Ok(RequestFormBody::Empty)
    );
    assert_eq!(
        get_request_body("DELETE", "application/json", br#"{"x":1}"#, &limits),
        Ok(RequestFormBody::Empty)
    );
    assert_eq!(
        get_request_body("PUT", "application/json", br#"{"x":1}"#, &limits),
        Ok(RequestFormBody::Empty)
    );
    let Ok(RequestFormBody::Json(DecodedJson::Parsed { body, repaired })) =
        get_request_body("POST", "application/json", br#"{"x":1}"#, &limits)
    else {
        panic!("a POST JSON body should parse");
    };
    assert!(!repaired);
    assert_eq!(body.get("x"), Some(&serde_json::json!(1)));
    assert_eq!(
        get_request_body("POST", "text/plain", b"x", &limits),
        Err(RequestFormError::UnsupportedContentType(
            "text/plain".to_owned()
        ))
    );
    assert_eq!(
        get_request_body("POST", "", b"x", &limits),
        Err(RequestFormError::UnsupportedContentType(String::new()))
    );

    let RequestFormBody::Form(urlencoded) =
        get_request_body("POST", "application/x-www-form-urlencoded", b"x=1", &limits).unwrap()
    else {
        panic!("a urlencoded body should parse as form fields");
    };
    assert_eq!(urlencoded.get("x"), Some(&FormValue::Text("1".to_owned())));

    let boundary = "multipart/form-data; boundary=test";
    let multipart_body =
        b"--test\r\nContent-Disposition: form-data; name=\"x\"\r\n\r\n1\r\n--test--\r\n";
    let RequestFormBody::Form(multipart) =
        get_request_body("POST", boundary, multipart_body, &limits).unwrap()
    else {
        panic!("a multipart body should parse as form fields");
    };
    assert_eq!(multipart.get("x"), Some(&FormValue::Text("1".to_owned())));

    let input = RequestBodyInput {
        method: "POST".to_owned(),
        route_path: "/v1/chat/completions".to_owned(),
        headers: BTreeMap::from([("content-type".to_owned(), boundary.to_owned())]),
        query_params: BTreeMap::new(),
        body: multipart_body.to_vec(),
        client_requested_model: None,
        cached_body: None,
        form_fields: None,
    };
    let result =
        super::super::request_body::read_request_body(&input, &Settings::default(), &limits)
            .unwrap();
    assert_eq!(result.body.get("x"), Some(&serde_json::json!("1")));
}

#[test]
fn multipart_boundary_variants_are_checked() {
    let limits = ApiLimits::default();
    let quoted =
        b"--quoted\r\nContent-Disposition: form-data; name=\"x\"\r\n\r\ny\r\n--quoted--\r\n";
    decode_request_form(quoted, b"multipart/form-data; boundary=\"quoted\"", &limits).unwrap();
    assert_eq!(
        decode_request_form(quoted, b"multipart/form-data", &limits),
        Err(RequestFormError::ContentType)
    );
    assert_eq!(
        decode_request_form(quoted, b"multipart/form-data; boundary=wrong", &limits),
        Err(RequestFormError::InvalidBody)
    );
}

#[test]
fn a_payload_that_starts_with_the_boundary_is_not_a_delimiter() {
    let payload = b"--b\r\nContent-Disposition: form-data; name=\"x\"\r\n\r\n--b--\r\n--b--\r\n";
    let parts = decode_request_form(payload, MULTIPART, &ApiLimits::default()).unwrap();
    let fields = get_form_data(&parts).unwrap();
    assert_eq!(fields.get("x"), Some(&FormValue::Text("--b--".to_owned())));
}

#[test]
fn form_part_count_is_bounded() {
    let limits = ApiLimits {
        max_json_bytes: 16 * 1024 * 1024,
        max_event_bytes: 1024 * 1024,
        max_items: 1,
    };
    assert_eq!(
        decode_request_form(b"a=1&b=2", URLENCODED, &limits),
        Err(RequestFormError::TooManyParts)
    );
    decode_request_form(b"a=1", URLENCODED, &limits).unwrap();
    let body = format!("{}{}--b--\r\n", text_part("a", "1"), text_part("b", "2"));
    assert_eq!(
        decode_request_form(body.as_bytes(), MULTIPART, &limits),
        Err(RequestFormError::TooManyParts)
    );
}

fn whisper_request() -> JsonObject {
    let mut request_data = JsonObject::new();
    request_data.insert("model".to_owned(), Json::Text("whisper".to_owned()));
    request_data
}

#[rstest]
#[case::within_a_byte(Some(1.0_f64), 0x000f_ffff, None)]
#[case::at_the_limit(Some(1.0_f64), 0x0010_0000, None)]
#[case::one_byte_over(Some(1.0_f64), 0x0010_0001, Some((400, "bad_request", "file", "400")))]
#[case::at_the_cap(None, 0x0100_0000, None)]
#[case::above_the_cap(None, 0x0100_0001, Some((413, "invalid_request_error", "file", "request_too_large")))]
fn compatibility_deviations(
    #[case] configured_limit: Option<f64>,
    #[case] bytes: u64,
    #[case] expected: Option<(u16, &str, &str, &str)>,
) {
    let router = configured_limit.map(|limit| FixedRouter {
        limit: UploadLimit::Configured(limit),
    });
    let router_names = match configured_limit {
        Some(_limit) => BTreeSet::from(["whisper".to_owned()]),
        None => BTreeSet::new(),
    };
    let mut request_data = whisper_request();
    let result = check_file_size_under_limit(UploadSizeCheck {
        request_data: &mut request_data,
        file_size: Some(bytes),
        router_names: &router_names,
        router: limits_of(router.as_ref()),
        limits: &ApiLimits::default(),
    });
    match (expected, result) {
        (None, Ok(outcome)) => assert!(outcome.proceeded),
        (Some(fields), Err(error)) => {
            let facts = error.facts();
            assert_eq!(
                (facts.status, facts.error_type, facts.param, facts.code),
                fields
            );
        }
        _other => panic!("the size check returned an unexpected outcome"),
    }
}

#[test]
fn loaded_limit_reaches_file_size_checking() {
    use betterlite_config::loader::ConfigLoader;
    use std::collections::BTreeMap;

    let loaded = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load("model_list:\n  - model_name: whisper\n    litellm_params:\n      max_file_size_mb: 1\n")
    .unwrap();
    let router = FixedRouter {
        limit: UploadLimit::Configured(loaded.sections.request_form.deployments[0].unwrap()),
    };
    let mut request_data = whisper_request();
    let error = check_file_size_under_limit(UploadSizeCheck {
        request_data: &mut request_data,
        file_size: Some(0x0010_0001),
        router_names: &BTreeSet::from(["whisper".to_owned()]),
        router: Some(&router),
        limits: &ApiLimits::default(),
    })
    .unwrap_err();
    assert_eq!(error.facts().status, 400);
}

const BOUNDARY_MISMATCH: &str = "multipart-mismatch";

#[test]
fn replays_corpus() {
    let corpus = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/request_form");
    let paths: Vec<_> = std::fs::read_dir(corpus).unwrap().collect();
    assert!(!paths.is_empty());
    for entry in paths {
        let path = entry.unwrap().path();
        let seed = std::fs::read(&path).unwrap();
        let header_len = usize::from(u16::from_be_bytes([seed[0], seed[1]]));
        let (header, body) = seed[2..].split_at(header_len);
        let result = decode_request_form(body, header, &ApiLimits::default());
        if path.file_stem().and_then(|stem| stem.to_str()) == Some(BOUNDARY_MISMATCH) {
            assert_eq!(result, Err(RequestFormError::InvalidBody));
        } else {
            result.unwrap();
        }
    }
}

#[rstest]
#[case(r#"name="file"; filename="report;v2.pdf""#, "file", "report;v2.pdf")]
#[case(r#"name="file"; filename="a\"b.pdf""#, "file", "a\"b.pdf")]
#[case(
    r#"name="file"; filename="C:\folder\report.pdf""#,
    "file",
    "report.pdf"
)]
#[case(r#"Name="file"; FILENAME="report.pdf""#, "file", "report.pdf")]
#[case(r#"name=""; filename="report.pdf""#, "", "report.pdf")]
fn multipart_disposition_parameters(
    #[case] parameters: &str,
    #[case] name: &str,
    #[case] filename: &str,
) {
    let body =
        format!("--b\r\nContent-Disposition: form-data; {parameters}\r\n\r\nabc\r\n--b--\r\n");
    let parts = decode_request_form(body.as_bytes(), MULTIPART, &ApiLimits::default()).unwrap();
    assert_eq!(parts.items.len(), 1);
    assert_eq!(parts.items[0].name, name);
    assert_eq!(
        parts.items[0].value,
        betterlite_core::api::request_form::values::PartValue::Upload(UploadFile {
            filename: filename.to_owned(),
            content_type: None,
            content: b"abc".to_vec(),
            size: Some(3),
        })
    );
}

#[rstest]
#[case("", "é")]
#[case("; charset=iso-8859-1", "é")]
#[case("; charset=windows-1251", "й")]
#[case("; charset=unknown", "é")]
fn multipart_charset_and_fallback(#[case] charset: &str, #[case] expected: &str) {
    use betterlite_core::api::request_form::values::PartValue;
    let body = b"--b\r\nContent-Disposition: form-data; name=\"\xe9\"\r\n\r\n\xe9\r\n--b\r\nContent-Disposition: form-data; name=\"file\"; filename=\"\xe9.pdf\"\r\n\r\n\xff\r\n--b--\r\n";
    let content_type = format!("multipart/form-data; boundary=b{charset}");
    let parts = decode_request_form(body, content_type.as_bytes(), &ApiLimits::default()).unwrap();
    assert_eq!(parts.items.len(), 2);
    assert_eq!(parts.items[0].name, expected);
    assert_eq!(parts.items[0].value, PartValue::Text(expected.to_owned()));
    assert_eq!(
        parts.items[1].value,
        PartValue::Upload(UploadFile {
            filename: format!("{expected}.pdf"),
            content_type: None,
            content: vec![255],
            size: Some(1),
        })
    );
}

#[test]
fn empty_multipart_body_has_no_fields() {
    assert!(
        decode_request_form(b"", MULTIPART, &ApiLimits::default())
            .unwrap()
            .items
            .is_empty()
    );
}

#[test]
fn an_unterminated_payload_boundary_is_content() {
    let body = b"--b\r\nContent-Disposition: form-data; name=\"x\"\r\n\r\n--b--";
    let mut items = super::multipart::raw_split::split_items(body, "b");
    assert_eq!(items.next().unwrap().unwrap().data, b"--b--");
    assert!(items.next().is_none());
}
