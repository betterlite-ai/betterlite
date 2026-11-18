#![cfg(test)]

use super::{RequestFormBody, RequestFormError, decode_request_form, get_request_body};
use betterlite_core::api::api_boundaries::ApiLimits;

#[test]
fn repeated_plain_last_wins() {
    let parts = decode_request_form(
        b"model=first&model=last",
        b"application/x-www-form-urlencoded",
        &ApiLimits::default(),
    )
    .unwrap();
    let fields = betterlite_core::api::request_form::values::get_form_data(&parts).unwrap();
    assert_eq!(
        fields["model"],
        betterlite_core::api::request_form::values::FormValue::Text("last".to_owned())
    );
}

#[test]
fn array_form_preserves_order() {
    let parts = decode_request_form(
        b"timestamp_granularities%5B%5D=word&timestamp_granularities%5B%5D=segment",
        b"application/x-www-form-urlencoded",
        &ApiLimits::default(),
    )
    .unwrap();
    let fields = betterlite_core::api::request_form::values::get_form_data(&parts).unwrap();
    assert_eq!(
        fields["timestamp_granularities"],
        betterlite_core::api::request_form::values::FormValue::List(vec![
            betterlite_core::api::request_form::values::FormValue::Text("word".to_owned()),
            betterlite_core::api::request_form::values::FormValue::Text("segment".to_owned()),
        ])
    );
}

#[test]
fn numeric_form_coercion() {
    use betterlite_core::api::request_form::{
        annotations::{FormAnnotation, numeric_form_fields},
        coercion::coerce_numeric_form_fields,
        values::FormValue,
    };
    let annotations = [("count".to_owned(), FormAnnotation::Int)]
        .into_iter()
        .collect();
    let numeric = numeric_form_fields(&annotations);
    let parts = decode_request_form(
        b"count=%201_000%20",
        b"application/x-www-form-urlencoded",
        &ApiLimits::default(),
    )
    .unwrap();
    let fields = betterlite_core::api::request_form::values::get_form_data(&parts).unwrap();
    assert_eq!(
        coerce_numeric_form_fields(&fields, &numeric)["count"],
        FormValue::Number(serde_json::Number::from(1000))
    );
}

#[test]
fn nested_form_metadata() {
    use betterlite_core::api::request_form::{
        metadata::{MetadataValue, extract_nested_form_metadata},
        values::{FormValue, get_form_data},
    };
    let parts = decode_request_form(
        b"litellm_metadata%5Ba%5D%5Bb%5D=v",
        b"application/x-www-form-urlencoded",
        &ApiLimits::default(),
    )
    .unwrap();
    let fields = get_form_data(&parts).unwrap();
    let (metadata, warnings) = extract_nested_form_metadata(&fields, "litellm_metadata[");
    assert!(warnings.is_empty());
    let Some(MetadataValue::Object(child)) = metadata.get("a") else {
        panic!("expected nested object");
    };
    assert_eq!(
        child.get("b"),
        Some(&MetadataValue::Leaf(FormValue::Text("v".to_owned())))
    );
}

#[test]
fn upload_size_unset() {
    use betterlite_core::api::request_form::file_size::{
        UploadSizeCheck, check_file_size_under_limit,
    };
    use betterlite_models::{decisions::Json, model_registration::JsonObject};
    use num_traits::ToPrimitive as _;
    use std::collections::BTreeMap;
    use std::collections::BTreeSet;
    let mut request_data = JsonObject::new();
    request_data.insert("model".to_owned(), Json::Text("whisper".to_owned()));
    let outcome = check_file_size_under_limit(UploadSizeCheck {
        request_data: &mut request_data,
        file_size: Some(1_048_576),
        router_names: &BTreeSet::new(),
        router: None,
        limits: &ApiLimits::default(),
    })
    .unwrap();
    assert!(outcome.proceeded);
}

#[test]
fn request_body_dispatch() {
    use betterlite_config::sections::request_body::Settings;
    use betterlite_core::api::request_body::RequestBodyInput;
    use std::collections::BTreeMap;

    let input = RequestBodyInput {
        method: "POST".to_owned(),
        route_path: "/v1/chat/completions".to_owned(),
        headers: BTreeMap::from([(
            "content-type".to_owned(),
            "multipart/form-data; boundary=test".to_owned(),
        )]),
        query_params: BTreeMap::new(),
        body: b"--test\r\nContent-Disposition: form-data; name=\"x\"\r\n\r\n1\r\n--test--\r\n"
            .to_vec(),
        client_requested_model: None,
        cached_body: None,
        form_fields: None,
    };
    let result = super::super::request_body::read_request_body(
        &input,
        &Settings::default(),
        &ApiLimits::default(),
    )
    .unwrap();
    assert_eq!(
        result.body.get("x"),
        Some(&serde_json::Value::String("1".to_owned()))
    );
    assert_eq!(
        get_request_body("GET", "application/json", b"garbage", &ApiLimits::default()),
        Ok(RequestFormBody::Empty)
    );
    assert!(matches!(
        get_request_body(
            "POST",
            "application/json",
            br#"{"x":1}"#,
            &ApiLimits::default()
        ),
        Ok(RequestFormBody::Json(_))
    ));
    assert_eq!(
        get_request_body("POST", "text/plain", b"x", &ApiLimits::default()),
        Err(RequestFormError::UnsupportedContentType(
            "text/plain".to_owned()
        ))
    );
    assert_eq!(
        decode_request_form(b"", b"text/plain", &ApiLimits::default()),
        Err(RequestFormError::ContentType)
    );
}

#[test]
fn multipart_boundary_variants_are_checked() {
    let limits = ApiLimits::default();
    let quoted =
        b"--quoted\r\nContent-Disposition: form-data; name=\"x\"\r\n\r\ny\r\n--quoted--\r\n";
    assert!(
        decode_request_form(quoted, b"multipart/form-data; boundary=\"quoted\"", &limits).is_ok()
    );
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
fn form_part_count_is_bounded() {
    let limits = ApiLimits {
        max_items: 1,
        ..ApiLimits::default()
    };
    assert_eq!(
        decode_request_form(b"a=1&b=2", b"application/x-www-form-urlencoded", &limits),
        Err(RequestFormError::TooManyParts)
    );
}

#[test]
fn compatibility_deviations() {
    use betterlite_core::api::request_form::file_size::{
        ModelUploadLimits, UploadLimit, UploadSizeCheck, UploadSizeError,
        check_file_size_under_limit,
    };
    use betterlite_models::{decisions::Json, model_registration::JsonObject};
    use std::collections::BTreeSet;

    struct Router(UploadLimit);
    impl ModelUploadLimits for Router {
        fn upload_limit_mebibytes(&self, _model: &str) -> UploadLimit {
            self.0
        }
    }

    let limits = ApiLimits::default();
    let loaded = betterlite_config::loader::ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load("model_list:\n  - model_name: whisper\n    litellm_params:\n      max_file_size_mb: 1\n")
    .unwrap();
    let configured_mb = loaded.sections.request_form.limit_for_index(0).unwrap();
    let router = Router(UploadLimit::Configured(configured_mb.to_f64().unwrap()));
    let router_names = BTreeSet::from(["whisper".to_owned()]);
    for bytes in [1_048_575, 1_048_576] {
        let mut request_data = JsonObject::new();
        request_data.insert("model".to_owned(), Json::Text("whisper".to_owned()));
        assert!(
            check_file_size_under_limit(UploadSizeCheck {
                request_data: &mut request_data,
                file_size: Some(bytes),
                router_names: &router_names,
                router: Some(&router),
                limits: &limits,
            })
            .is_ok()
        );
    }
    let mut request_data = JsonObject::new();
    request_data.insert("model".to_owned(), Json::Text("whisper".to_owned()));
    let error = check_file_size_under_limit(UploadSizeCheck {
        request_data: &mut request_data,
        file_size: Some(1_048_577),
        router_names: &router_names,
        router: Some(&router),
        limits: &limits,
    })
    .unwrap_err();
    assert_eq!(
        error,
        UploadSizeError::TooLarge {
            size_mb: 1_048_577.0 / 1_048_576.0,
            limit_mb: 1.0
        }
    );
    assert_eq!(error.facts().status, 400);

    for bytes in [16_777_216, 16_777_217] {
        let mut request_data = JsonObject::new();
        let result = check_file_size_under_limit(UploadSizeCheck {
            request_data: &mut request_data,
            file_size: Some(bytes),
            router_names: &BTreeSet::new(),
            router: None,
            limits: &limits,
        });
        if bytes == 16_777_216 {
            assert!(result.is_ok());
        } else {
            assert!(matches!(
                result,
                Err(UploadSizeError::AboveSafetyCap { .. })
            ));
        }
    }
}

#[test]
fn replays_corpus() {
    for seed in [
        include_bytes!("../../../fuzz/corpus/request_form/urlencoded").as_slice(),
        include_bytes!("../../../fuzz/corpus/request_form/multipart").as_slice(),
        include_bytes!("../../../fuzz/corpus/request_form/multipart-quoted").as_slice(),
        include_bytes!("../../../fuzz/corpus/request_form/multipart-mismatch").as_slice(),
    ] {
        let header_len = usize::from(u16::from_be_bytes([seed[0], seed[1]]));
        let (header, body) = seed[2..].split_at(header_len);
        let _ = decode_request_form(body, header, &ApiLimits::default());
    }
}
