#![cfg(test)]

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
use serde_json::Number;

use super::{
    RequestedOptions, provider_request_params, requested_optional_params, requested_params,
};
use crate::api::bfl::edit::media::SuppliedMedia;
use crate::api::video_generation_call::VideoGenerationRequest;
use crate::api::video_generation_call::error::VideoGenerationCallError;
use crate::api::video_generation_call::testing::TestingConfig;
use crate::api::video_records::VideoCreateOptionalRequestParams;
use crate::api::videos_http_transport::video_config::Params;

fn empty_options() -> VideoCreateOptionalRequestParams {
    VideoCreateOptionalRequestParams {
        input_reference: None,
        image: None,
        parameters: None,
        model: None,
        resolution: None,
        seconds: None,
        size: None,
        characters: None,
        user: None,
        extra_headers: None,
        extra_body: None,
    }
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn text_map(entries: &[(&str, &str)]) -> Json {
    let mut fields = IndexMap::new();
    for (name, value) in entries {
        fields.insert((*name).to_owned(), text(value));
    }
    Json::Object(fields)
}

fn request() -> VideoGenerationRequest {
    VideoGenerationRequest::new("a prompt".to_owned())
}

#[test]
fn removes_null_and_owned_entries() {
    let mut call = request();
    call.seconds = Some("8".to_owned());
    call.size = Some("720x1280".to_owned());
    call.custom_llm_provider = Some("openai".to_owned());
    call.kwargs
        .insert("mock_response".to_owned(), text("mocked"));
    call.kwargs
        .insert("api_base".to_owned(), text("http://local"));
    call.kwargs.insert("size".to_owned(), Json::Null);
    call.kwargs.insert("seed".to_owned(), Json::Null);

    let params = requested_params(&mut call);
    assert_eq!(params, Params::from([("seconds".to_owned(), text("8"))]));

    let mut nullable_call = request();
    nullable_call.seconds = Some("8".to_owned());
    nullable_call
        .kwargs
        .insert("mock_response".to_owned(), text("mocked"));
    nullable_call.kwargs.insert("size".to_owned(), Json::Null);
    let options = requested_optional_params(&mut nullable_call).unwrap().video;
    assert_eq!(
        options,
        VideoCreateOptionalRequestParams {
            seconds: Some("8".to_owned()),
            ..empty_options()
        }
    );
}

#[test]
fn a_loose_keyword_beats_the_named_argument() {
    let mut call = request();
    call.size = Some("720x1280".to_owned());
    call.kwargs.insert("size".to_owned(), text("1080x1920"));
    call.kwargs.insert("user".to_owned(), text("loose-user"));

    let options = requested_optional_params(&mut call).unwrap().video;
    assert_eq!(options.size.as_deref(), Some("1080x1920"));
    assert_eq!(options.user.as_deref(), Some("loose-user"));
}

#[test]
fn a_loose_null_removes_the_named_value() {
    let mut call = request();
    call.size = Some("720x1280".to_owned());
    call.kwargs.insert("size".to_owned(), Json::Null);

    let options = requested_optional_params(&mut call).unwrap().video;
    assert_eq!(options.size, None);
}

#[test]
fn extra_body_merges_and_its_flat_entries_beat_everything() {
    let mut call = request();
    call.size = Some("720x1280".to_owned());
    call.kwargs.insert("size".to_owned(), text("1080x1920"));
    call.extra_body = Some(IndexMap::from([
        ("size".to_owned(), text("480x854")),
        ("seed".to_owned(), text("top")),
    ]));
    call.kwargs.insert(
        "extra_body".to_owned(),
        text_map(&[
            ("size", "640x480"),
            ("seed", "loose"),
            ("mock_response", "owned"),
        ]),
    );

    let options = requested_optional_params(&mut call).unwrap().video;
    assert_eq!(options.size.as_deref(), Some("640x480"));
    assert_eq!(
        options.extra_body,
        Some(IndexMap::from([
            ("size".to_owned(), "640x480".to_owned()),
            ("seed".to_owned(), "loose".to_owned()),
        ]))
    );
    assert_eq!(
        call.extra_body,
        Some(IndexMap::from([
            ("size".to_owned(), text("480x854")),
            ("seed".to_owned(), text("top")),
        ]))
    );
}

#[test]
fn a_loose_extra_body_hides_the_named_one() {
    let mut call = request();
    call.extra_body = Some(IndexMap::from([("user".to_owned(), text("named-user"))]));
    call.kwargs
        .insert("extra_body".to_owned(), text_map(&[("size", "480x854")]));

    let options = requested_optional_params(&mut call).unwrap().video;
    assert_eq!(options.user, None);
    assert_eq!(options.size.as_deref(), Some("480x854"));
    assert_eq!(
        options.extra_body,
        Some(IndexMap::from([("size".to_owned(), "480x854".to_owned())]))
    );
}

#[test]
fn extra_body_merges_null_entries_are_skipped() {
    let mut call = request();
    call.kwargs.insert(
        "extra_body".to_owned(),
        Json::Object(IndexMap::from([
            ("size".to_owned(), Json::Null),
            ("seed".to_owned(), text("7")),
            ("api_key".to_owned(), text("owned")),
        ])),
    );

    let options = requested_optional_params(&mut call).unwrap().video;
    assert_eq!(
        options.extra_body,
        Some(IndexMap::from([("seed".to_owned(), "7".to_owned())]))
    );
}

#[test]
fn the_timeout_goes_last_and_the_loose_map_loses_extra_body() {
    let mut call = request();
    call.timeout = Some(0.0_f64);
    call.extra_body = Some(IndexMap::from([("timeout".to_owned(), text("7"))]));
    call.kwargs
        .insert("extra_body".to_owned(), text_map(&[("timeout", "5")]));
    call.kwargs
        .insert("timeout".to_owned(), Json::Number(Number::from(9_u64)));

    let params = requested_params(&mut call);
    assert!(!params.contains_key("timeout"));
    assert!(!call.kwargs.contains_key("extra_body"));
    assert_eq!(
        call.extra_body,
        Some(IndexMap::from([("timeout".to_owned(), text("7"))]))
    );
}

#[test]
fn an_extra_body_that_is_not_a_map_is_ignored_but_removed() {
    let mut call = request();
    call.kwargs
        .insert("extra_body".to_owned(), text("not a map"));

    let options = requested_optional_params(&mut call).unwrap().video;
    assert_eq!(options, empty_options());
    assert!(!call.kwargs.contains_key("extra_body"));
}

#[test]
fn a_flat_input_reference_fills_the_typed_field() {
    let mut call = request();
    call.kwargs.insert(
        "extra_body".to_owned(),
        text_map(&[("input_reference", "frame.png")]),
    );

    let options = requested_optional_params(&mut call).unwrap().video;
    assert_eq!(
        options.input_reference,
        Some(SuppliedMedia::Text("frame.png".to_owned()))
    );
}

#[test]
fn a_value_the_declared_type_cannot_hold_is_refused_by_name() {
    let mut call = request();
    call.kwargs
        .insert("seconds".to_owned(), Json::Number(Number::from(8_u64)));
    let error = requested_optional_params(&mut call).unwrap_err();
    let VideoGenerationCallError::FieldType { field, expected } = error else {
        panic!("a wrong value must be refused by name");
    };
    assert_eq!(field, "seconds");
    assert_eq!(expected, "a string");
}

#[test]
fn provider_request_params_maps_then_overrides_from_extra_body() {
    let mapped = Params::from([
        ("seconds".to_owned(), text("8")),
        ("size".to_owned(), text("720x1280")),
        ("extra_body".to_owned(), text_map(&[("size", "ignored")])),
    ]);
    let config = TestingConfig::returning(mapped);
    let options = VideoCreateOptionalRequestParams {
        seconds: Some("8".to_owned()),
        size: Some("720x1280".to_owned()),
        extra_body: Some(IndexMap::from([
            ("size".to_owned(), "480x854".to_owned()),
            ("seed".to_owned(), "7".to_owned()),
        ])),
        ..empty_options()
    };

    let params = provider_request_params(
        &config,
        "sora-2",
        true,
        &RequestedOptions {
            video: options.clone(),
            extra_query: None,
        },
    );
    assert_eq!(params.get("seconds"), Some(&text("8")));
    assert_eq!(params.get("size"), Some(&text("480x854")));
    assert_eq!(params.get("seed"), Some(&text("7")));
    assert!(!params.contains_key("extra_body"));

    let calls = config.calls();
    assert_eq!(calls.len(), 1_usize);
    assert_eq!(calls[0].model, "sora-2");
    assert!(calls[0].drop_params);
    assert_eq!(calls[0].options, options);
}

#[test]
fn provider_request_params_drops_an_empty_extra_body_key() {
    let config = TestingConfig::returning(Params::from([
        ("size".to_owned(), text("720x1280")),
        ("extra_body".to_owned(), Json::Object(IndexMap::new())),
    ]));
    let options = VideoCreateOptionalRequestParams {
        extra_body: Some(IndexMap::new()),
        ..empty_options()
    };

    let params = provider_request_params(
        &config,
        "sora-2",
        false,
        &RequestedOptions {
            video: options,
            extra_query: None,
        },
    );
    assert_eq!(params.get("size"), Some(&text("720x1280")));
    assert!(!params.contains_key("extra_body"));
}

#[rstest]
#[case::named(None, Some(text_map(&[("after", "named")])))]
#[case::loose(Some(text_map(&[("after", "loose")])), Some(text_map(&[("after", "loose")])))]
#[case::empty(Some(text_map(&[])), Some(text_map(&[])))]
#[case::null(Some(Json::Null), None)]
fn query_survives_projection_and_mapping(
    #[case] loose: Option<Json>,
    #[case] expected: Option<Json>,
) {
    let mut call = request();
    call.extra_query = Some(Params::from([("after".to_owned(), text("named"))]));
    if let Some(value) = loose {
        call.kwargs.insert("extra_query".to_owned(), value);
    }
    let options = requested_optional_params(&mut call).unwrap();
    let mapped = provider_request_params(&TestingConfig::default(), "sora-2", false, &options);
    assert_eq!(mapped.get("extra_query"), expected.as_ref());
}

#[test]
fn a_query_of_the_wrong_type_names_the_field() {
    let mut call = request();
    call.kwargs
        .insert("extra_query".to_owned(), text("not a map"));
    let error = requested_optional_params(&mut call).unwrap_err();
    assert_eq!(error.to_string(), "extra_query must be a map");
}
