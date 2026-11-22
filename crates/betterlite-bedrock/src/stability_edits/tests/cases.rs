use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_models::image_cost::error::ImageCostError;
use betterlite_models::price_lookup::PriceRecord;
use betterlite_models::tiered_pricing::Scalar;
use betterlite_openai::api::image_contracts::records::{ImageData, ImageHttpResponse};
use serde_json::from_str;

use super::support::{
    INPAINT, NOW, Prices, STYLE_TRANSFER, UNKNOWN, integer, json, options, price_record,
    provider_message, text, upstream, with_bytes,
};
use crate::bedrock_settings::ErrorHeaders;
use crate::stability_edits::environment::{
    COMPLETE_URL, USE_MULTIPART_FORM_DATA, get_error_class, validate_environment,
};
use crate::stability_edits::error::StabilityEditError;
use crate::stability_edits::params::{SUPPORTED_OPENAI_PARAMS, map_openai_params};
use crate::stability_edits::reply::{COST_HEADER, ImageEditReply, transform_image_edit_response};
use crate::stability_edits::request::transform_image_edit_request;
use crate::stability_edits::selection::is_stability_edit_model;
use crate::stability_edits::types::{EditValue, ImageInput};

pub(super) fn stability_edit_model_selection() {
    for model in [
        None,
        Some(""),
        Some("stable-diffusion"),
        Some("inpaint"),
        Some("stability."),
        Some("stability.sd3-5-large-v1:0"),
        Some("stability.stable-diffusion-xl-v1"),
    ] {
        assert!(!is_stability_edit_model(model), "{model:?}");
    }
    for model in [
        "stability.stable-conservative-upscale-v1:0",
        "STABILITY.STABLE-OUTPAINT-V1:0",
        "stability.stable-image-inpaint-v1:0",
        "stability.stable-image-erase-object-v1:0",
        "stability.stable-image-remove-background-v1:0",
        "stability.stable-image-search-recolor-v1:0",
        "stability.stable-image-search-replace-v1:0",
        "stability.stable-image-control-sketch-v1:0",
        "stability.stable-image-control-structure-v1:0",
        "stability.stable-image-style-guide-v1:0",
        "x-stability.stable-style-transfer-v1:0",
    ] {
        assert!(is_stability_edit_model(Some(model)), "{model}");
    }
}

pub(super) fn image_and_mask_fields() {
    image_parts();
    mask_parts();
}

fn image_parts() {
    let plain = transform_image_edit_request(INPAINT, None, None, &options(&[]))
        .expect("an image is optional");
    assert_eq!(plain.body, json(r#"{"output_format":"png"}"#));
    assert!(plain.files.is_empty());
    let blank = transform_image_edit_request(INPAINT, Some(""), None, &options(&[]))
        .expect("an empty prompt is omitted");
    assert_eq!(blank.body, json(r#"{"output_format":"png"}"#));
    let prompted = transform_image_edit_request(INPAINT, Some("cat"), None, &options(&[]))
        .expect("the prompt is kept");
    assert_eq!(
        prompted.body,
        json(r#"{"output_format":"png","prompt":"cat"}"#)
    );
    assert_eq!(
        image_body(INPAINT, &ImageInput::Bytes(b"hello".to_vec())),
        json(r#"{"output_format":"png","image":"aGVsbG8="}"#)
    );
    assert_eq!(
        image_body(INPAINT, &ImageInput::Text("aGVsbG8=".to_owned())),
        json(r#"{"output_format":"png","image":"aGVsbG8="}"#)
    );
    assert_eq!(
        image_body(STYLE_TRANSFER, &ImageInput::Text("abc".to_owned())),
        json(r#"{"output_format":"png","init_image":"abc"}"#)
    );
    assert_eq!(
        image_body(
            "STABILITY.STABLE-STYLE-TRANSFER-V1:0",
            &ImageInput::Text("abc".to_owned())
        ),
        json(r#"{"output_format":"png","init_image":"abc"}"#)
    );
}

fn image_body(model: &str, image: &ImageInput) -> Json {
    let request =
        transform_image_edit_request(model, None, Some(image), &options(&[])).expect("the image");
    request.body
}

fn mask_parts() {
    assert_eq!(
        masked(("mask", text("maskb64"))),
        json(r#"{"output_format":"png","image":"img","mask":"maskb64"}"#)
    );
    assert_eq!(
        masked(("mask", json(r#"["m1","m2"]"#))),
        json(r#"{"output_format":"png","image":"img","mask":"m1"}"#)
    );
    assert_eq!(
        masked(("mask", json("[]"))),
        json(r#"{"output_format":"png","image":"img","mask":"[]"}"#)
    );
    assert_eq!(
        masked(("mask", json("[5]"))),
        json(r#"{"output_format":"png","image":"img","mask":"5"}"#)
    );
    assert_eq!(
        masked(("style_image", text("style"))),
        json(r#"{"output_format":"png","image":"img","style_image":"style"}"#)
    );
    let bytes = with_bytes(options(&[]), "mask", b"mb");
    let request =
        transform_image_edit_request(INPAINT, None, None, &bytes).expect("the mask encodes");
    assert_eq!(
        request.body,
        json(r#"{"output_format":"png","mask":"bWI="}"#)
    );
    let overridden = transform_image_edit_request(
        STYLE_TRANSFER,
        None,
        Some(&ImageInput::Text("img".to_owned())),
        &options(&[("init_image", text("explicit"))]),
    )
    .expect("the option overwrites init_image");
    assert_eq!(
        overridden.body,
        json(r#"{"output_format":"png","init_image":"explicit"}"#)
    );
}

fn masked(option: (&str, Json)) -> Json {
    let request = transform_image_edit_request(
        INPAINT,
        None,
        Some(&ImageInput::Text("img".to_owned())),
        &options(&[option]),
    )
    .expect("the mask encodes");
    request.body
}

pub(super) fn native_edit_options() {
    conversions();
    preserved();
}

fn converted(entries: &[(&str, Json)]) -> Json {
    transform_image_edit_request(INPAINT, None, None, &options(entries))
        .expect("the numeric fields convert")
        .body
}

fn field(body: &Json, name: &str) -> Json {
    let Json::Object(fields) = body else {
        return Json::Null;
    };
    fields.get(name).cloned().unwrap_or(Json::Null)
}

fn conversions() {
    conversion_values();
    int_big();
    non_finite();
}

fn int_big() {
    let body = converted(&[("left", json("123456789012345678"))]);
    assert_eq!(field(&body, "left"), json("123456789012345678"));
}

fn conversion_values() {
    let cases = [
        ("left", text("12"), json("12")),
        ("left", text("-0"), json("0")),
        ("right", json("12.7"), json("12")),
        ("right", text("-"), text("-")),
        ("up", Json::Bool(true), json("1")),
        ("up", text("_5"), text("_5")),
        ("down", text("bad"), text("bad")),
        ("down", Json::Bool(false), json("0")),
        ("down", text("5_"), text("5_")),
        ("seed", text("1_0"), json("10")),
        ("seed", json("-0.5"), json("0")),
        ("strength", text("0.5"), json("0.5")),
        ("creativity", integer(2), json("2.0")),
        ("control_strength", text("bad"), text("bad")),
        ("grow_mask", json("0.25"), json("0.25")),
        ("fidelity", text("0.75"), json("0.75")),
        ("composition_fidelity", Json::Bool(true), json("1.0")),
        ("style_strength", text("0.25"), json("0.25")),
        ("change_strength", integer(2), json("2.0")),
        ("change_strength", Json::Bool(false), json("0.0")),
        ("negative_prompt", text("no"), text("no")),
        ("aspect_ratio", text("1:1"), text("1:1")),
        ("output_format", text("jpeg"), text("jpeg")),
        ("model", text("m"), text("m")),
        ("mode", text("x"), text("x")),
        ("style_preset", text("y"), text("y")),
        ("select_prompt", text("s"), text("s")),
        ("search_prompt", text("q"), text("q")),
        ("quality", text("high"), Json::Null),
    ];
    for (name, value, expected) in cases {
        let body = converted(&[(name, value)]);
        assert_eq!(field(&body, name), expected, "{name}");
    }
}

fn non_finite() {
    let error =
        transform_image_edit_request(INPAINT, None, None, &options(&[("strength", text("inf"))]))
            .expect_err("a non-finite float has no JSON form");
    assert!(matches!(error, StabilityEditError::Field { .. }));
}

fn preserved() {
    let request = transform_image_edit_request(
        INPAINT,
        None,
        None,
        &options(&[
            ("_n", integer(2)),
            ("_response_format", text("url")),
            ("left", Json::Null),
            ("right", json("[1]")),
            ("up", json(r#"{"a":1}"#)),
            ("quality", text("high")),
            ("init_image", Json::Null),
        ]),
    )
    .expect("internal, null and unknown keys are skipped");
    assert_eq!(
        request.body,
        json(r#"{"output_format":"png","right":[1],"up":{"a":1}}"#)
    );
}

pub(super) fn stability_artifact_results() {
    success_images();
    empty_images();
    error_markers();
    finish_reasons();
}

fn edit_reply(model: &str, status: u16, body: &str) -> Result<ImageEditReply, StabilityEditError> {
    let upstream = upstream(status, body);
    transform_image_edit_response(model, &upstream, NOW, &Prices::catalog())
}

fn priced_reply(record: PriceRecord) -> Result<ImageEditReply, StabilityEditError> {
    let upstream = upstream(200, r#"{"images":["x"]}"#);
    transform_image_edit_response(INPAINT, &upstream, NOW, &Prices::new(&[(INPAINT, record)]))
}

fn success_images() {
    let reply = edit_reply(
        INPAINT,
        200,
        r#"{"images":["aGVsbG8=","",null,"d29ybGQ="]}"#,
    )
    .expect("the images decode");
    assert_eq!(reply.response.created, NOW);
    assert_eq!(
        reply.response.data,
        Some(vec![
            ImageData {
                b64_json: Some("aGVsbG8=".to_owned()),
                revised_prompt: None,
                url: None,
            },
            ImageData {
                b64_json: Some("d29ybGQ=".to_owned()),
                revised_prompt: None,
                url: None,
            },
        ])
    );
    assert_eq!(
        reply.additional_headers.get(COST_HEADER),
        Some(&json("0.07"))
    );
    let style =
        edit_reply(STYLE_TRANSFER, 200, r#"{"images":["x"]}"#).expect("the style image decodes");
    assert_eq!(
        style.additional_headers.get(COST_HEADER),
        Some(&json("0.08"))
    );
}

fn empty_images() {
    for body in [
        "{}",
        r#"{"images":[]}"#,
        r#"{"images":["",null,false,0]}"#,
        r#"{"images":[{}]}"#,
    ] {
        let reply = edit_reply(INPAINT, 200, body).expect("no images is allowed");
        assert_eq!(reply.response.data, Some(Vec::new()), "{body}");
        assert_eq!(
            reply.additional_headers.get(COST_HEADER),
            Some(&json("0.07")),
            "{body}"
        );
    }
    let failed_status = edit_reply(INPAINT, 500, r#"{"images":["x"]}"#)
        .expect("a non-200 status alone is not an error");
    assert_eq!(
        failed_status.response.data,
        Some(vec![ImageData {
            b64_json: Some("x".to_owned()),
            revised_prompt: None,
            url: None,
        }])
    );
}

fn error_markers() {
    let error = edit_reply(INPAINT, 200, r#"{"errors":"bad","images":["x"]}"#)
        .expect_err("the errors key wins over images");
    assert_eq!(error.status(), Some(200));
    assert_eq!(provider_message(&error), "Bedrock Stability error: bad");
    let StabilityEditError::Provider { error: carried, .. } = &error else {
        panic!("expected a provider error");
    };
    assert_eq!(
        carried.response_headers,
        vec![("x-a".to_owned(), "b".to_owned())]
    );
    let null_error = edit_reply(INPAINT, 500, r#"{"errors":null}"#)
        .expect_err("a present null key still raises");
    assert_eq!(null_error.status(), Some(500));
    assert_eq!(
        provider_message(&null_error),
        "Bedrock Stability error: None"
    );
    let empty = edit_reply(INPAINT, 502, r#"{"errors":[]}"#)
        .expect_err("an empty errors list still raises");
    assert_eq!(empty.status(), Some(502));
}

fn finish_reasons() {
    let first = edit_reply(
        INPAINT,
        200,
        r#"{"images":["x"],"finish_reasons":["ERROR","OTHER"]}"#,
    )
    .expect_err("the first finish reason raises");
    assert_eq!(first.status(), Some(400));
    assert_eq!(provider_message(&first), "Bedrock Stability error: ERROR");
    let container = edit_reply(INPAINT, 200, r#"{"images":["x"],"finish_reasons":"E"}"#)
        .expect_err("a string container answers its first character");
    assert_eq!(container.status(), Some(400));
    assert_eq!(provider_message(&container), "Bedrock Stability error: E");
    let nested = edit_reply(INPAINT, 200, r#"{"images":["x"],"finish_reasons":[[1]]}"#)
        .expect_err("a nonempty list reason is truthy");
    assert_eq!(nested.status(), Some(400));
    for body in [
        r#"{"images":["x"],"finish_reasons":[null,"ERROR"]}"#,
        r#"{"images":["x"],"finish_reasons":["","ERROR"]}"#,
        r#"{"images":["x"],"finish_reasons":[]}"#,
        r#"{"images":["x"],"finish_reasons":null}"#,
        r#"{"images":["x"],"finish_reasons":""}"#,
        r#"{"images":["x"],"finish_reasons":[[]]}"#,
        r#"{"images":["x"],"finish_reasons":[false]}"#,
        r#"{"images":["x"],"finish_reasons":0}"#,
        r#"{"images":["x"],"finish_reasons":false}"#,
        r#"{"images":["x"],"finish_reasons":{}}"#,
    ] {
        let reply = edit_reply(INPAINT, 200, body).expect("a falsey first reason is ignored");
        assert_eq!(
            reply.response.data.as_ref().map(Vec::len),
            Some(1),
            "{body}"
        );
    }
}

// The catalog round trip serializes a declared Decimal as text, which is the shape a real reply reads.
fn price_values() {
    let catalog_text = price_record(&[("output_cost_per_image", Scalar::Text("0.07".to_owned()))]);
    let text_header = priced_reply(catalog_text).expect("a text price reads as a decimal");
    assert_eq!(
        text_header.additional_headers.get(COST_HEADER),
        Some(&json("0.07"))
    );
    for (padded, expected) in [("  0.5  ", "0.5"), ("5E-2", "0.05")] {
        let padded_price =
            price_record(&[("output_cost_per_image", Scalar::Text(padded.to_owned()))]);
        let padded_header = priced_reply(padded_price).expect("a text price reads as a decimal");
        assert_eq!(
            padded_header.additional_headers.get(COST_HEADER),
            Some(&json(expected)),
            "{padded}"
        );
    }
    for (flag, expected) in [(true, "1.0"), (false, "0.0")] {
        let flag_price = price_record(&[("output_cost_per_image", Scalar::Bool(flag))]);
        let flag_header = priced_reply(flag_price).expect("a bool price reads as a number");
        assert_eq!(
            flag_header.additional_headers.get(COST_HEADER),
            Some(&json(expected)),
            "{flag}"
        );
    }
}

pub(super) fn filtered_result_error() {
    parse_failures();
    record_failures();
    price_values();
    price_failures();
}

fn parse_failures() {
    let error = edit_reply(INPAINT, 422, "not json")
        .expect_err("a body that is not JSON becomes a Bedrock error");
    assert_eq!(error.status(), Some(422));
    assert!(
        provider_message(&error).starts_with("Error parsing Bedrock Stability response: "),
        "{}",
        provider_message(&error)
    );
    let StabilityEditError::Provider { error: carried, .. } = &error else {
        panic!("expected a provider error");
    };
    assert_eq!(
        carried.response_headers,
        vec![("x-a".to_owned(), "b".to_owned())]
    );
}

fn record_failures() {
    assert_record_failure("null");
    assert_record_failure("[]");
    assert_record_failure(r#""hello""#);
    assert_record_failure(r#"{"images":[5]}"#);
    assert_record_failure(r#"{"images":[{"a":1}]}"#);
    assert_record_failure(r#"{"images":"x"}"#);
    assert_record_failure(r#"{"finish_reasons":{"a":1}}"#);
}

fn assert_record_failure(body: &str) {
    let error =
        edit_reply(INPAINT, 200, body).expect_err("a parsed record of the wrong shape fails");
    assert_eq!(error.status(), None, "{body}");
    assert!(matches!(&error, StabilityEditError::Reply(_)), "{body}");
}

fn price_failures() {
    let upstream = upstream(200, r#"{"images":["x"]}"#);
    text_price_failure(&upstream);
    shape_price_failures();
    price_header_cases(&upstream);
}

fn unmapped_model_price() {
    let reply = edit_reply(UNKNOWN, 200, r#"{"images":["x"]}"#)
        .expect("an unmapped model still returns its image");
    assert_eq!(
        reply.response.data,
        Some(vec![ImageData {
            b64_json: Some("x".to_owned()),
            revised_prompt: None,
            url: None,
        }])
    );
    assert_eq!(
        reply.additional_headers.get(COST_HEADER),
        Some(&json("0.0"))
    );
}

fn text_price_failure(upstream: &ImageHttpResponse) {
    let text_price = Prices::new(&[(
        INPAINT,
        price_record(&[("output_cost_per_image", Scalar::Text("nope".to_owned()))]),
    )]);
    let error = transform_image_edit_response(INPAINT, upstream, NOW, &text_price)
        .expect_err("a non-numeric price fails");
    assert_eq!(error.status(), None);
    assert!(matches!(
        error,
        StabilityEditError::Price(ImageCostError::Rate(_))
    ));
}

fn shape_price_failures() {
    for body in [
        r#"{"output_cost_per_image":[1]}"#,
        r#"{"output_cost_per_image":{"amount":1}}"#,
    ] {
        let record: PriceRecord = from_str(body).expect("the fixture is a price record");
        let shape_error =
            priced_reply(record).expect_err("a list or object price is not a decimal");
        assert!(
            matches!(
                shape_error,
                StabilityEditError::Price(ImageCostError::WrongFieldType { .. })
            ),
            "{body}"
        );
    }
}

fn price_header_cases(upstream: &ImageHttpResponse) {
    unmapped_model_price();
    let null_price = Prices::new(&[(
        INPAINT,
        price_record(&[("output_cost_per_image", Scalar::Null)]),
    )]);
    let null_reply = transform_image_edit_response(INPAINT, upstream, NOW, &null_price)
        .expect("a null price omits the header");
    assert_eq!(null_reply.additional_headers.get(COST_HEADER), None);

    // the declared default applies to an absent key, where LiteLLM's get_model_info has already filled a None
    let absent = Prices::new(&[(INPAINT, price_record(&[]))]);
    let absent_reply = transform_image_edit_response(INPAINT, upstream, NOW, &absent)
        .expect("an absent price is zero");
    assert_eq!(
        absent_reply.additional_headers.get(COST_HEADER),
        Some(&json("0.0"))
    );
}

pub(super) fn stability_edit_url() {
    let multipart = USE_MULTIPART_FORM_DATA;
    assert!(!multipart);
    assert_eq!(COMPLETE_URL, "bedrock://image-edit");
    header_checks();
    error_class();
}

fn header_checks() {
    let json_type = BTreeMap::from([("Content-Type".to_owned(), "application/json".to_owned())]);
    assert_eq!(validate_environment(None), json_type);
    assert_eq!(validate_environment(Some(BTreeMap::new())), json_type);
    let lowercase = BTreeMap::from([("content-type".to_owned(), "text/plain".to_owned())]);
    let validated = validate_environment(Some(lowercase));
    assert_eq!(
        validated.get("Content-Type").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(
        validated.get("content-type").map(String::as_str),
        Some("text/plain")
    );
    let exact = BTreeMap::from([("Content-Type".to_owned(), "text/plain".to_owned())]);
    assert_eq!(validate_environment(Some(exact.clone())), exact);
    let other = BTreeMap::from([("A".to_owned(), "b".to_owned())]);
    let preserved = validate_environment(Some(other));
    assert_eq!(preserved.get("A").map(String::as_str), Some("b"));
    assert_eq!(preserved.len(), 2);
}

fn error_class() {
    let error = get_error_class(
        "boom".to_owned(),
        418,
        vec![("h".to_owned(), "v".to_owned())],
    );
    assert_eq!(error.status(), Some(418));
    assert_eq!(provider_message(&error), "boom");
    let StabilityEditError::Provider { error: carried, .. } = &error else {
        panic!("expected a provider error");
    };
    assert_eq!(carried.error.status_code, 418);
    assert_eq!(
        carried.headers,
        Some(ErrorHeaders::Http(vec![("h".to_owned(), "v".to_owned())]))
    );
}

pub(super) fn stability_option_mapping() {
    assert_eq!(
        SUPPORTED_OPENAI_PARAMS,
        ["n", "size", "response_format", "mask"]
    );
    size_mapping();
    passthrough();
    unsupported();
}

fn size_mapping() {
    for (size, ratio) in [
        ("1024x1024", "1:1"),
        ("512x512", "1:1"),
        ("256x256", "1:1"),
        ("1792x1024", "16:9"),
        ("1024x1792", "9:16"),
    ] {
        let mapped = map_openai_params(&options(&[("size", text(size))]), INPAINT, false)
            .expect("a known size maps");
        assert_eq!(
            mapped.get("aspect_ratio"),
            Some(&EditValue::Json(text(ratio))),
            "{size}"
        );
        assert_eq!(mapped.get("size"), None, "{size}");
    }
    for size in [text("999x999"), Json::Null, integer(5)] {
        let mapped = map_openai_params(&options(&[("size", size)]), INPAINT, false)
            .expect("an unknown size adds nothing");
        assert!(mapped.is_empty());
    }
    let list = map_openai_params(
        &options(&[("size", json(r#"["1024x1024"]"#))]),
        INPAINT,
        false,
    )
    .expect_err("a list size is unhashable");
    assert!(matches!(list, StabilityEditError::Field { .. }));
    let object = map_openai_params(&options(&[("size", json(r#"{"a":1}"#))]), INPAINT, false)
        .expect_err("an object size is unhashable");
    assert!(matches!(object, StabilityEditError::Field { .. }));
}

fn passthrough() {
    let mapped = map_openai_params(
        &options(&[
            ("n", integer(2)),
            ("response_format", text("b64_json")),
            ("mask", text("abc")),
        ]),
        INPAINT,
        false,
    )
    .expect("supported params stay");
    assert_eq!(
        mapped.keys().map(String::as_str).collect::<Vec<_>>(),
        ["mask", "_n", "_response_format"]
    );
    assert_eq!(mapped.get("_n"), Some(&EditValue::Json(integer(2))));
    assert_eq!(
        mapped.get("_response_format"),
        Some(&EditValue::Json(text("b64_json")))
    );
    assert_eq!(mapped.get("mask"), Some(&EditValue::Json(text("abc"))));
    let ordered = map_openai_params(
        &options(&[
            ("quality", text("high")),
            ("size", text("1024x1024")),
            ("n", integer(1)),
        ]),
        INPAINT,
        true,
    )
    .expect("drop_params keeps the initial copy");
    assert_eq!(
        ordered.keys().map(String::as_str).collect::<Vec<_>>(),
        ["quality", "aspect_ratio", "_n"]
    );
}

fn unsupported() {
    let error = map_openai_params(&options(&[("quality", text("high"))]), INPAINT, false)
        .expect_err("an unsupported key fails without drop_params");
    assert!(matches!(
        &error,
        StabilityEditError::UnsupportedParameter { parameter, model }
            if parameter == "quality" && model == INPAINT
    ));
    assert_eq!(
        error.to_string(),
        "Parameter quality is not supported for model stability.stable-image-inpaint-v1:0. Supported parameters are ['n', 'size', 'response_format', 'mask']. Set drop_params=True to drop unsupported parameters."
    );
    let dropped = map_openai_params(&options(&[("quality", text("high"))]), INPAINT, true)
        .expect("drop_params keeps the initial copy");
    assert_eq!(dropped.get("quality"), Some(&EditValue::Json(text("high"))));
    for name in ["aspect_ratio", "init_image", "_n"] {
        let failure = map_openai_params(&options(&[(name, text("x"))]), INPAINT, false)
            .expect_err("a native or internal key is not an OpenAI param");
        assert!(
            matches!(&failure, StabilityEditError::UnsupportedParameter { .. }),
            "{name}"
        );
    }
}
