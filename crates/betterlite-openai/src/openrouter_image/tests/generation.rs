#![cfg(test)]

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::SecretString;

use crate::api::image_contracts::records::{
    ImageInputTokens, ImageParams, ImageResponse, ImageUsage,
};
use crate::openrouter_image::decode::OpenRouterImageUsage;
use crate::openrouter_image::generation::{
    complete_url as generation_url, get_openrouter_image_generation_config as generation_config,
    request_body as generation_request_body, transform_response as generation_response,
    validate_environment as generation_headers,
};
use crate::openrouter_image::shared::{ImageConfigError, ImageHiddenParams, apply_usage};

use super::{COST_HEADER, CREATED, b64, decimal, headers, json, ok, params, raw, text, url_image};

#[test]
fn generation_params_map_size_and_quality_into_image_config() {
    let supplied = params(r#"{"size": "1536x1024", "quality": "high"}"#);
    let mapped = generation_config("openrouter/model")
        .map_params(&supplied, ImageParams::new(), false)
        .expect("the params map");
    assert_eq!(
        mapped,
        params(r#"{"image_config": {"aspect_ratio": "3:2", "image_size": "4K"}}"#)
    );
    assert_eq!(
        supplied,
        params(r#"{"size": "1536x1024", "quality": "high"}"#)
    );
}

#[test]
fn generation_params_copy_unsupported_keys_unless_dropped() {
    let supplied = params(r#"{"n": 2, "size": "1024x1024", "style": "vivid"}"#);
    let kept = generation_config("m")
        .map_params(&supplied, params(r#"{"user": "u"}"#), false)
        .expect("the params map");
    assert_eq!(
        kept,
        params(
            r#"{"user": "u", "n": 2, "image_config": {"aspect_ratio": "1:1"}, "style": "vivid"}"#
        )
    );
    let dropped = generation_config("m")
        .map_params(&supplied, ImageParams::new(), true)
        .expect("the params map");
    assert_eq!(
        dropped,
        params(r#"{"n": 2, "image_config": {"aspect_ratio": "1:1"}}"#)
    );
}

#[test]
fn generation_params_keep_an_existing_image_config_and_its_keys() {
    let optional = params(r#"{"image_config": {"aspect_ratio": "16:9", "image_size": "2K"}}"#);
    let mapped = generation_config("m")
        .map_params(&params(r#"{"quality": "low"}"#), optional, false)
        .expect("the params map");
    assert_eq!(
        mapped,
        params(r#"{"image_config": {"aspect_ratio": "16:9", "image_size": "1K"}}"#)
    );
}

#[test]
fn generation_params_add_no_image_config_for_an_unknown_quality() {
    let mapped = generation_config("m")
        .map_params(
            &params(r#"{"quality": "ultra"}"#),
            params(r#"{"n": 1}"#),
            false,
        )
        .expect("the params map");
    assert_eq!(mapped, params(r#"{"n": 1}"#));
}

#[rstest]
#[case::number(r#"{"size": 1024}"#)]
#[case::null(r#"{"size": null}"#)]
#[case::list(r#"{"size": []}"#)]
fn generation_params_reject_a_size_that_is_not_a_string(#[case] supplied: &str) {
    let error = generation_config("m")
        .map_params(&params(supplied), ImageParams::new(), false)
        .expect_err("size must be a string");
    assert!(matches!(error, ImageConfigError::Size));
}

#[rstest]
#[case::number(r#"{"quality": 1}"#)]
#[case::null(r#"{"quality": null}"#)]
fn generation_params_reject_a_quality_that_is_not_a_string(#[case] supplied: &str) {
    let error = generation_config("m")
        .map_params(&params(supplied), ImageParams::new(), false)
        .expect_err("quality must be a string");
    assert!(matches!(error, ImageConfigError::Quality));
}

#[rstest]
#[case::text(r#"{"image_config": "square"}"#)]
#[case::number(r#"{"image_config": 1}"#)]
#[case::list(r#"{"image_config": []}"#)]
#[case::null(r#"{"image_config": null}"#)]
fn generation_params_reject_an_image_config_that_is_not_an_object(#[case] optional: &str) {
    let error = generation_config("m")
        .map_params(&params(r#"{"size": "1024x1024"}"#), params(optional), false)
        .expect_err("image_config must be an object");
    assert!(matches!(error, ImageConfigError::ImageConfig));
}

#[rstest]
#[case::default(None, "https://openrouter.ai/api/v1/chat/completions")]
#[case::empty(Some(""), "https://openrouter.ai/api/v1/chat/completions")]
#[case::bare_host(Some("https://proxy.test"), "https://proxy.test/chat/completions")]
#[case::versioned(
    Some("https://proxy.test/api/v1"),
    "https://proxy.test/api/v1/chat/completions"
)]
#[case::trailing_slashes(
    Some("https://proxy.test/api/v1//"),
    "https://proxy.test/api/v1/chat/completions"
)]
#[case::chat_endpoint(
    Some("https://proxy.test/chat/completions"),
    "https://proxy.test/chat/completions"
)]
#[case::doubled(
    Some("https://proxy.test/chat/completions/"),
    "https://proxy.test/chat/completions/chat/completions"
)]
fn generation_urls_end_at_chat_completions(#[case] base: Option<&str>, #[case] expected: &str) {
    assert_eq!(generation_url(base), expected);
}

#[test]
fn generation_headers_send_bearer_none_without_a_key() {
    let chosen = generation_headers(headers(&[("X-Extra", "1")]), None, None, None);
    assert_eq!(
        chosen,
        headers(&[("X-Extra", "1"), ("Authorization", "Bearer None")])
    );
}

#[test]
fn generation_headers_take_the_first_non_empty_key() {
    let api = SecretString::from("api-key");
    let global = SecretString::from("global-key");
    let environment = SecretString::from("environment-key");
    let empty = SecretString::from("");
    let chosen = generation_headers(
        headers(&[]),
        Some(&empty),
        Some(&global),
        Some(&environment),
    );
    assert_eq!(chosen["Authorization"], "Bearer global-key");
    let chosen = generation_headers(headers(&[]), Some(&api), Some(&global), Some(&environment));
    assert_eq!(chosen["Authorization"], "Bearer api-key");
    let chosen = generation_headers(headers(&[]), None, None, Some(&environment));
    assert_eq!(chosen["Authorization"], "Bearer environment-key");
    let chosen = generation_headers(headers(&[]), None, None, Some(&empty));
    assert_eq!(chosen["Authorization"], "Bearer None");
}

#[test]
fn generation_requests_are_chat_messages() {
    let optional = params(
        r#"{"n": 2, "image_config": {"aspect_ratio": "1:1"}, "model": "ignored", "messages": [], "modalities": []}"#,
    );
    let body = generation_request_body("openrouter/model", "Draw a cat", &optional);
    assert_eq!(
        body,
        params(
            r#"{"model": "openrouter/model", "messages": [{"role": "user", "content": "Draw a cat"}], "n": 2, "image_config": {"aspect_ratio": "1:1"}}"#
        )
    );
}

#[test]
fn generation_requests_keep_an_empty_prompt() {
    let body = generation_request_body("m", "", &ImageParams::new());
    assert_eq!(
        body,
        params(r#"{"model": "m", "messages": [{"role": "user", "content": ""}]}"#)
    );
}

#[test]
fn generation_responses_read_images_and_usage() {
    let raw = ok(
        r#"{"model": "reply/model", "choices": [{"message": {"images": [{"image_url": {"url": "data:image/png;base64,AAAA"}}, {"image_url": {"url": "https://example.test/b.png"}}]}}], "usage": {"prompt_tokens": 7, "total_tokens": 9, "completion_tokens_details": {"image_tokens": 2}, "cost": 0.25, "cost_details": {"upstream": 0.2}}}"#,
    );
    let (response, hidden) = generation_response(
        &raw,
        ImageResponse::default(),
        ImageHiddenParams::default(),
        "requested/model",
    )
    .expect("the reply transforms");
    assert_eq!(
        response.data,
        Some(vec![b64("AAAA"), url_image("https://example.test/b.png")])
    );
    assert_eq!(
        response.usage,
        Some(ImageUsage {
            input_tokens: 7,
            input_tokens_details: ImageInputTokens {
                image_tokens: 0,
                text_tokens: 7,
            },
            output_tokens: 2,
            total_tokens: 9,
        })
    );
    assert_eq!(hidden.model, Some(text("reply/model")));
    let expected_details = IndexMap::from([(String::from("upstream"), json("0.2"))]);
    assert_eq!(hidden.response_cost_details, Some(expected_details));
    let Some(Json::Number(cost)) = hidden.additional_headers.get(COST_HEADER) else {
        panic!("the cost header is set");
    };
    assert_eq!(cost.as_f64(), Some(0.25_f64));
}

#[test]
fn generation_responses_append_to_existing_data() {
    let existing = ImageResponse {
        created: CREATED,
        data: Some(vec![url_image("https://example.test/first.png")]),
        usage: None,
    };
    let raw = ok(
        r#"{"choices": [{"message": {"images": [{"image_url": {"url": "data:image/png;base64,BBBB"}}]}}]}"#,
    );
    let (response, hidden) = generation_response(
        &raw,
        existing,
        ImageHiddenParams::default(),
        "requested/model",
    )
    .expect("the reply transforms");
    assert_eq!(response.created, CREATED);
    assert_eq!(
        response.data,
        Some(vec![
            url_image("https://example.test/first.png"),
            b64("BBBB")
        ])
    );
    assert_eq!(hidden.model, Some(text("requested/model")));
}

#[test]
fn generation_responses_without_images_have_an_empty_data_list() {
    let (response, hidden) = generation_response(
        &ok("{}"),
        ImageResponse::default(),
        ImageHiddenParams::default(),
        "m",
    )
    .expect("the reply transforms");
    assert_eq!(response.data, Some(Vec::new()));
    assert_eq!(response.usage, None);
    assert_eq!(
        hidden,
        ImageHiddenParams {
            additional_headers: IndexMap::new(),
            response_cost_details: None,
            model: Some(text("m")),
        }
    );
}

#[test]
fn generation_responses_keep_existing_hidden_params_and_merge_cost_details() {
    let hidden = ImageHiddenParams {
        additional_headers: IndexMap::from([(String::from("x-existing"), text("kept"))]),
        response_cost_details: Some(IndexMap::from([(String::from("upstream"), json("0.1"))])),
        model: Some(text("old/model")),
    };
    let raw = ok(r#"{"usage": {"cost": 2, "cost_details": {"upstream": 0.3, "cache": 0.05}}}"#);
    let (_, hidden) = generation_response(&raw, ImageResponse::default(), hidden, "m")
        .expect("the reply transforms");
    assert_eq!(
        hidden.additional_headers.get("x-existing"),
        Some(&text("kept"))
    );
    let expected_details = IndexMap::from([
        (String::from("upstream"), json("0.3")),
        (String::from("cache"), json("0.05")),
    ]);
    assert_eq!(hidden.response_cost_details, Some(expected_details));
    assert_eq!(hidden.model, Some(text("m")));
    let Some(Json::Number(cost)) = hidden.additional_headers.get(COST_HEADER) else {
        panic!("the cost header is set");
    };
    assert_eq!(cost.as_f64(), Some(2.0_f64));
}

#[test]
fn generation_responses_without_usage_keep_the_existing_usage() {
    let usage = ImageUsage {
        input_tokens: 1,
        input_tokens_details: ImageInputTokens {
            image_tokens: 0,
            text_tokens: 1,
        },
        output_tokens: 0,
        total_tokens: 1,
    };
    let response = ImageResponse {
        created: 0,
        data: None,
        usage: Some(usage.clone()),
    };
    let (response, hidden) =
        generation_response(&ok("{}"), response, ImageHiddenParams::default(), "m")
            .expect("the reply transforms");
    assert_eq!(response.usage, Some(usage));
    assert!(hidden.additional_headers.is_empty());
    assert_eq!(hidden.model, Some(text("m")));
}

#[test]
fn generation_responses_ignore_an_empty_usage() {
    let (response, hidden) = generation_response(
        &ok(r#"{"usage": {}}"#),
        ImageResponse::default(),
        ImageHiddenParams::default(),
        "m",
    )
    .expect("the reply transforms");
    assert_eq!(response.usage, None);
    assert!(hidden.additional_headers.is_empty());
}

#[test]
fn generation_responses_set_the_hidden_model_from_the_reply_or_the_request() {
    let cases = [
        (r#"{"model": "reply/model"}"#, Some(text("reply/model"))),
        (r#"{"model": null}"#, Some(Json::Null)),
        ("{}", Some(text("requested/model"))),
    ];
    let models: Vec<Option<Json>> = cases
        .iter()
        .map(|(body, _expected)| {
            let (_, hidden) = generation_response(
                &ok(body),
                ImageResponse::default(),
                ImageHiddenParams::default(),
                "requested/model",
            )
            .expect("the reply transforms");
            hidden.model
        })
        .collect();
    let expected: Vec<Option<Json>> = cases.iter().map(|(_, model)| model.clone()).collect();
    assert_eq!(models, expected);
}

#[test]
fn generation_responses_read_null_completion_details_as_zero() {
    let raw = ok(
        r#"{"usage": {"prompt_tokens": 3, "total_tokens": 5, "completion_tokens_details": null}}"#,
    );
    let (response, _hidden) = generation_response(
        &raw,
        ImageResponse::default(),
        ImageHiddenParams::default(),
        "m",
    )
    .expect("null takes the default");
    assert_eq!(
        response.usage,
        Some(ImageUsage {
            input_tokens: 3,
            input_tokens_details: ImageInputTokens {
                image_tokens: 0,
                text_tokens: 3,
            },
            output_tokens: 0,
            total_tokens: 5,
        })
    );
}

#[test]
fn generation_responses_store_a_zero_cost() {
    let raw = ok(r#"{"usage": {"cost": 0}}"#);
    let (_, hidden) = generation_response(
        &raw,
        ImageResponse::default(),
        ImageHiddenParams::default(),
        "m",
    )
    .expect("the reply transforms");
    let Some(Json::Number(cost)) = hidden.additional_headers.get(COST_HEADER) else {
        panic!("the cost header is set");
    };
    assert_eq!(cost.as_f64(), Some(0.0_f64));
}

#[test]
fn generation_responses_reject_a_usage_field_of_the_wrong_type() {
    let raw = ok(r#"{"usage": {"completion_tokens_details": {"image_tokens": "2"}}}"#);
    let error = generation_response(
        &raw,
        ImageResponse::default(),
        ImageHiddenParams::default(),
        "m",
    )
    .expect_err("the usage field is wrong");
    assert_eq!(error.status_code, 500);
    assert!(error.headers.is_empty());
    assert!(
        error.message.starts_with(
            "Error transforming OpenRouter image generation response: usage.completion_tokens_details.image_tokens: invalid type"
        ),
        "{}",
        error.message
    );
}

#[test]
fn generation_usage_writes_a_decimal_cost_as_a_number() {
    let usage = OpenRouterImageUsage {
        prompt_tokens: 3,
        total_tokens: 4,
        output_image_tokens: 0,
        input_image_tokens: 0,
        cost: Some(decimal("0.0000001")),
        cost_details: IndexMap::new(),
    };
    let mut response = ImageResponse::default();
    let mut hidden = ImageHiddenParams::default();
    apply_usage(&usage, &mut response, &mut hidden, 0).expect("the usage applies");
    assert_eq!(
        response.usage,
        Some(ImageUsage {
            input_tokens: 3,
            input_tokens_details: ImageInputTokens {
                image_tokens: 0,
                text_tokens: 3,
            },
            output_tokens: 0,
            total_tokens: 4,
        })
    );
    assert_eq!(
        hidden.additional_headers.get(COST_HEADER),
        Some(&json("0.0000001"))
    );
    assert!(hidden.response_cost_details.is_none());
}

#[test]
fn an_integral_cost_is_written_as_a_float() {
    let usage = OpenRouterImageUsage {
        prompt_tokens: 1,
        total_tokens: 2,
        output_image_tokens: 1,
        input_image_tokens: 0,
        cost: Some(decimal("2")),
        cost_details: IndexMap::new(),
    };
    let mut hidden = ImageHiddenParams::default();
    apply_usage(&usage, &mut ImageResponse::default(), &mut hidden, 0).expect("the usage applies");
    assert_eq!(
        hidden.additional_headers.get(COST_HEADER),
        Some(&json("2.0"))
    );
}

#[test]
fn generation_responses_skip_an_empty_cost_details() {
    let raw = ok(r#"{"usage": {"prompt_tokens": 1, "cost_details": {}}}"#);
    let (response, hidden) = generation_response(
        &raw,
        ImageResponse::default(),
        ImageHiddenParams::default(),
        "m",
    )
    .expect("the reply transforms");
    assert!(hidden.response_cost_details.is_none());
    assert!(hidden.additional_headers.is_empty());
    let Some(usage) = response.usage else {
        panic!("the usage is set");
    };
    assert_eq!(usage.input_tokens, 1);
}

#[test]
fn generation_parse_failures_keep_the_raw_status_and_headers() {
    let raw = raw(502, headers(&[("X-Raw", "1")]), "<html>");
    let error = generation_response(
        &raw,
        ImageResponse::default(),
        ImageHiddenParams::default(),
        "m",
    )
    .expect_err("the body is not JSON");
    assert_eq!(error.status_code, 502);
    assert_eq!(error.headers, headers(&[("X-Raw", "1")]));
    assert!(
        error
            .message
            .starts_with("Error parsing OpenRouter response: ")
    );
}

#[test]
fn generation_shape_failures_are_a_500_with_no_headers() {
    let raw = raw(502, headers(&[("X-Raw", "1")]), r#"{"choices": {}}"#);
    let error = generation_response(
        &raw,
        ImageResponse::default(),
        ImageHiddenParams::default(),
        "m",
    )
    .expect_err("choices must be a list");
    assert_eq!(error.status_code, 500);
    assert!(error.headers.is_empty());
    assert!(
        error.message.starts_with(
            "Error transforming OpenRouter image generation response: choices: invalid type"
        ),
        "{}",
        error.message
    );
}
