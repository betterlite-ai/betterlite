use betterlite_openai::api::image_contracts::records::{ImageInputTokens, ImageUsage};
use serde_json::Error as JsonError;

use crate::canvas_results::{COST_HEADER, CanvasReplyError, CanvasResultError};

use super::support::{
    MODEL, NOW, OTHER_MODEL, image, json, priced, provider_error, record, unpriced,
};

pub(super) fn canvas_images_array() {
    parses_json_before_the_status_check();
    keeps_truthy_images_in_order();
    rejects_other_statuses();
}

fn parses_json_before_the_status_check() {
    let error = unpriced(500, "not json").expect_err("a body that is not JSON is a parse error");
    assert_eq!(error.status(), Some(500));
    assert_eq!(
        provider_error(&error).response_headers,
        vec![("x-a".to_owned(), "b".to_owned())]
    );
    let message = &provider_error(&error).error.message;
    assert!(
        message.starts_with("Error parsing Nova Canvas image edit response: "),
        "{message}"
    );
}

fn keeps_truthy_images_in_order() {
    let body = r#"{"images":["aGVsbG8=","",null,"d29ybGQ="],"created":7,"usage":{"total_tokens":9},"size":"1024x1024","model":"upstream"}"#;
    let reply = unpriced(200, body).expect("the images decode");
    assert_eq!(reply.response.created, NOW);
    // the body's own usage is dropped and the fresh ImageResponse keeps LiteLLM's zero-filled default
    assert_eq!(
        reply.response.usage,
        Some(ImageUsage {
            input_tokens: 0,
            input_tokens_details: ImageInputTokens {
                image_tokens: 0,
                text_tokens: 0,
            },
            output_tokens: 0,
            total_tokens: 0,
        })
    );
    assert_eq!(
        reply.response.data,
        Some(vec![image("aGVsbG8="), image("d29ybGQ=")])
    );
    assert!(reply.additional_headers.is_empty());
}

fn rejects_other_statuses() {
    for status in [199_u16, 201_u16, 500_u16] {
        let error =
            unpriced(status, r#"{"images":["eA=="]}"#).expect_err("only status 200 builds a reply");
        assert_eq!(error.status(), Some(status));
        assert_eq!(
            provider_error(&error).response_headers,
            vec![("x-a".to_owned(), "b".to_owned())]
        );
    }
}

pub(super) fn canvas_error_object() {
    other_statuses_report_the_original_response();
    error_fields_fail_without_images();
    message_fields_fail_without_images();
    truthy_images_suppress_error_fields();
}

fn other_statuses_report_the_original_response() {
    for body in [r#"{"errors":"bad","images":["eA=="]}"#, r#""boom""#] {
        let error = unpriced(400, body).expect_err("a non-200 status is an error");
        assert_eq!(error.status(), Some(400));
        let message = &provider_error(&error).error.message;
        assert!(
            message.starts_with("Nova Canvas image edit error: "),
            "{message}"
        );
    }
    let error = unpriced(201, r#"{"images":["eA=="]}"#).expect_err("status 201 is not 200");
    assert_eq!(error.status(), Some(201));
}

fn error_fields_fail_without_images() {
    let error = unpriced(200, r#"{"errors":"bad"}"#).expect_err("an errors key is an error");
    assert_eq!(error.status(), Some(200));
    assert_eq!(
        provider_error(&error).error.message,
        "Nova Canvas image edit error: bad"
    );
    assert_eq!(
        provider_error(&error).response_headers,
        vec![("x-a".to_owned(), "b".to_owned())]
    );
    assert!(provider_error(&error).headers.is_some());
    for (body, expected) in [
        (r#"{"errors":null}"#, "Nova Canvas image edit error: None"),
        (r#"{"errors":""}"#, "Nova Canvas image edit error: "),
        (r#"{"errors":[]}"#, "Nova Canvas image edit error: object"),
        (r#"{"errors":0}"#, "Nova Canvas image edit error: 0"),
        (
            r#"{"images":[],"errors":"bad"}"#,
            "Nova Canvas image edit error: bad",
        ),
    ] {
        let failure = unpriced(200, body).expect_err("an errors key is an error");
        assert_eq!(provider_error(&failure).error.message, expected, "{body}");
    }
}

fn message_fields_fail_without_images() {
    for (body, expected) in [
        (r#"{"message":"m"}"#, "Nova Canvas image edit error: m"),
        (
            r#"{"message":"m","error":"e"}"#,
            "Nova Canvas image edit error: m",
        ),
        (
            r#"{"message":"","error":"e"}"#,
            "Nova Canvas image edit error: e",
        ),
        (
            r#"{"message":0,"error":"e"}"#,
            "Nova Canvas image edit error: e",
        ),
        (r#"{"error":"e"}"#, "Nova Canvas image edit error: e"),
        (r#"{"message":5}"#, "Nova Canvas image edit error: 5"),
        (r#"{"message":true}"#, "Nova Canvas image edit error: True"),
    ] {
        let error = unpriced(200, body).expect_err("a truthy message is an error");
        assert_eq!(error.status(), Some(200));
        assert_eq!(provider_error(&error).error.message, expected, "{body}");
    }
}

fn truthy_images_suppress_error_fields() {
    for body in [
        r#"{"images":[""],"errors":"bad"}"#,
        r#"{"images":[""],"message":"m"}"#,
        r#"{"images":[null],"errors":"bad"}"#,
    ] {
        let error = unpriced(200, body).expect_err("the empty filtered list is still an error");
        assert_eq!(error.status(), Some(200));
        assert_eq!(
            provider_error(&error).error.message,
            "Nova Canvas image edit returned no images",
            "{body}"
        );
    }
    for body in [
        r#"{"images":["eA=="],"errors":"bad","message":"m"}"#,
        r#"{"images":["eA=="],"finish_reasons":["E"]}"#,
    ] {
        let reply = unpriced(200, body).expect("truthy images suppress the error fields");
        assert_eq!(reply.response.data, Some(vec![image("eA==")]), "{body}");
    }
}

pub(super) fn canvas_empty_images() {
    empty_data_raises_at_the_original_status();
    error_fields_outrank_the_empty_list();
}

fn empty_data_raises_at_the_original_status() {
    for body in [
        "{}",
        r#"{"images":[]}"#,
        r#"{"images":["",null,false,0]}"#,
        r#"{"images":[{}]}"#,
        r#"{"images":[[]]}"#,
        r#"{"message":null,"error":null}"#,
    ] {
        let error = unpriced(200, body).expect_err("an empty filtered list is an error");
        assert_eq!(error.status(), Some(200), "{body}");
        assert_eq!(
            provider_error(&error).error.message,
            "Nova Canvas image edit returned no images",
            "{body}"
        );
        assert_eq!(
            provider_error(&error).response_headers,
            vec![("x-a".to_owned(), "b".to_owned())],
            "{body}"
        );
    }
}

fn error_fields_outrank_the_empty_list() {
    for (body, expected) in [
        (
            r#"{"images":[],"errors":"e"}"#,
            "Nova Canvas image edit error: e",
        ),
        (
            r#"{"images":[],"errors":null}"#,
            "Nova Canvas image edit error: None",
        ),
        (
            r#"{"images":[],"message":"m"}"#,
            "Nova Canvas image edit error: m",
        ),
    ] {
        let error = unpriced(200, body).expect_err("the marked field wins");
        assert_eq!(provider_error(&error).error.message, expected, "{body}");
    }
    let failed = unpriced(400, r#"{"images":[]}"#).expect_err("the status wins");
    assert_eq!(failed.status(), Some(400));
    let also_failed = unpriced(201, "{}").expect_err("the status wins");
    assert_eq!(also_failed.status(), Some(201));
}

pub(super) fn canvas_malformed_reply() {
    raw_json_failures_are_wrapped();
    the_decoder_failure_stays_in_the_chain();
    parsed_shapes_fail_at_their_fields();
    a_repeated_declared_key_is_not_the_object_it_reads();
    a_non_200_status_precedes_the_shape_checks();
}

fn the_decoder_failure_stays_in_the_chain() {
    let error = unpriced(500, "not json").expect_err("bytes that are not JSON are wrapped");
    let source = std::error::Error::source(&error).expect("the decoder failure is the source");
    let decoder = source
        .downcast_ref::<CanvasReplyError>()
        .expect("the source is the decoder failure");
    let cause = std::error::Error::source(decoder).expect("the serde error stays attached");
    assert!(cause.downcast_ref::<JsonError>().is_some());
}

fn raw_json_failures_are_wrapped() {
    for (status, body) in [
        (200_u16, "not json"),
        (200_u16, r#"{"images":"#),
        (200_u16, ""),
        (422_u16, r#"{"images":"#),
    ] {
        let error = unpriced(status, body).expect_err("bytes that are not JSON are wrapped");
        assert_eq!(error.status(), Some(status), "{body}");
        let message = &provider_error(&error).error.message;
        assert!(
            message.starts_with("Error parsing Nova Canvas image edit response: "),
            "{body}: {message}"
        );
    }
}

fn parsed_shapes_fail_at_their_fields() {
    for body in ["null", "[]", r#""hello""#, "5", "true"] {
        let error = unpriced(200, body).expect_err("a parsed non-object fails later");
        assert_eq!(error.status(), None, "{body}");
        assert!(
            matches!(
                &error,
                CanvasResultError::Reply(CanvasReplyError::NotAnObject)
            ),
            "{body}"
        );
        let message = error.to_string();
        assert!(!message.starts_with("Error parsing"), "{body}: {message}");
    }
    for body in [
        r#"{"images":5}"#,
        r#"{"images":"ab"}"#,
        r#"{"images":["x",7]}"#,
        r#"{"images":[{"a":1}]}"#,
        r#"{"images":[1.5]}"#,
        r#"{"images":[["a"]]}"#,
    ] {
        let error = unpriced(200, body).expect_err("a truthy non-string image fails");
        assert_eq!(error.status(), None, "{body}");
        assert!(
            matches!(
                &error,
                CanvasResultError::Reply(CanvasReplyError::Field { field })
                    if field.as_str() == "images"
            ),
            "{body}"
        );
    }
}

fn a_repeated_declared_key_is_not_the_object_it_reads() {
    let error = unpriced(200, r#"{"images":["eA=="],"images":["eQ=="]}"#)
        .expect_err("a repeated declared key fails the typed reply parse");
    assert_eq!(error.status(), None);
    assert!(matches!(
        &error,
        CanvasResultError::Reply(CanvasReplyError::NotAnObject)
    ));
}

fn a_non_200_status_precedes_the_shape_checks() {
    for body in ["null", "[1]"] {
        let error = unpriced(500, body).expect_err("a non-200 status is an error");
        assert_eq!(error.status(), Some(500), "{body}");
        let message = &provider_error(&error).error.message;
        assert!(
            message.starts_with("Nova Canvas image edit error: "),
            "{body}: {message}"
        );
    }
}

pub(super) fn canvas_response_cost() {
    the_price_times_the_retained_count();
    the_product_stays_decimal();
    scalar_prices_read_like_float();
    a_missing_price_is_zero();
    unusable_prices_suppress_the_header();
    an_unmapped_model_keeps_its_images();
}

fn the_price_times_the_retained_count() {
    let reply = priced(
        MODEL,
        record(r#"{"output_cost_per_image": 0.07}"#),
        200,
        r#"{"images":["eA==","","eQ=="],"size":"1024x1024"}"#,
    )
    .expect("the images decode");
    assert_eq!(
        reply.additional_headers.get(COST_HEADER),
        Some(&json("0.14"))
    );
    assert_eq!(
        reply.response.data,
        Some(vec![image("eA=="), image("eQ==")])
    );
    assert_eq!(reply.response.usage, Some(ImageUsage::default()));
    let wider = priced(
        OTHER_MODEL,
        record(r#"{"output_cost_per_image": 0.25}"#),
        200,
        r#"{"images":["eA==","eQ==","eQ==","eQ=="]}"#,
    )
    .expect("the other model's images decode");
    assert_eq!(
        wider.additional_headers.get(COST_HEADER),
        Some(&json("1.0"))
    );
}

fn the_product_stays_decimal() {
    let reply = priced(
        MODEL,
        record(r#"{"output_cost_per_image": 0.07}"#),
        200,
        r#"{"images":["eA==","eQ==","eQ=="]}"#,
    )
    .expect("the images decode");
    assert_eq!(
        reply.additional_headers.get(COST_HEADER),
        Some(&json("0.21"))
    );
}

fn scalar_prices_read_like_float() {
    for (record_source, images, expected) in [
        (
            r#"{"output_cost_per_image": "0.07"}"#,
            r#"{"images":["eA==","eQ=="]}"#,
            "0.14",
        ),
        (
            r#"{"output_cost_per_image": 5}"#,
            r#"{"images":["eA=="]}"#,
            "5.0",
        ),
        (
            r#"{"output_cost_per_image": true}"#,
            r#"{"images":["eA=="]}"#,
            "1.0",
        ),
        (
            r#"{"output_cost_per_image": false}"#,
            r#"{"images":["eA==","eQ=="]}"#,
            "0.0",
        ),
        (
            r#"{"output_cost_per_image": 0}"#,
            r#"{"images":["eA=="]}"#,
            "0.0",
        ),
    ] {
        let reply = priced(MODEL, record(record_source), 200, images).expect("the images decode");
        assert_eq!(
            reply.additional_headers.get(COST_HEADER),
            Some(&json(expected)),
            "{record_source}"
        );
    }
}

fn a_missing_price_is_zero() {
    for record_source in ["{}", r#"{"litellm_provider":"bedrock"}"#] {
        let reply = priced(MODEL, record(record_source), 200, r#"{"images":["eA=="]}"#)
            .expect("the images decode");
        assert_eq!(
            reply.additional_headers.get(COST_HEADER),
            Some(&json("0.0")),
            "{record_source}"
        );
        assert_eq!(reply.response.data, Some(vec![image("eA==")]));
    }
}

fn unusable_prices_suppress_the_header() {
    for record_source in [
        r#"{"output_cost_per_image": null}"#,
        r#"{"output_cost_per_image": "abc"}"#,
        r#"{"output_cost_per_image": [1]}"#,
        r#"{"output_cost_per_image": {"rate": 1}}"#,
        r#"{"output_cost_per_image": "inf"}"#,
    ] {
        let reply = priced(MODEL, record(record_source), 200, r#"{"images":["eA=="]}"#)
            .expect("the images decode");
        assert!(
            reply.additional_headers.get(COST_HEADER).is_none(),
            "{record_source}"
        );
        assert_eq!(
            reply.response.data,
            Some(vec![image("eA==")]),
            "{record_source}"
        );
    }
}

fn an_unmapped_model_keeps_its_images() {
    let reply =
        unpriced(200, r#"{"images":["eA=="]}"#).expect("an unmapped model still returns its image");
    assert!(reply.additional_headers.get(COST_HEADER).is_none());
    assert_eq!(reply.response.data, Some(vec![image("eA==")]));
    assert_eq!(reply.response.created, NOW);
}
