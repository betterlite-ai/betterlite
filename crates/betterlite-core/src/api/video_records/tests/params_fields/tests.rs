#![cfg(test)]

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;

use crate::api::bfl::edit::media::SuppliedMedia;
use crate::api::video_records::VideoCreateOptionalRequestParams;

use super::fixture::object;

#[test]
fn optional_params_carry_every_declared_field() {
    let params = VideoCreateOptionalRequestParams {
        input_reference: Some(SuppliedMedia::Text("image.png".to_owned())),
        image: Some(Json::Object(object(&[(
            "gcsUri",
            Json::Text("gs://bucket/image.png".to_owned()),
        )]))),
        parameters: Some(object(&[("seed", Json::Number(Number::from(7_i64)))])),
        model: Some("sora-2".to_owned()),
        resolution: Some("1280x720".to_owned()),
        seconds: Some("8".to_owned()),
        size: Some("1280x720".to_owned()),
        characters: Some(vec![IndexMap::from([(
            "id".to_owned(),
            "char_123".to_owned(),
        )])]),
        user: Some("user-1".to_owned()),
        extra_headers: Some(IndexMap::from([(
            "x-api-key".to_owned(),
            "value".to_owned(),
        )])),
        extra_body: Some(IndexMap::from([(
            "custom_llm_provider".to_owned(),
            "openai".to_owned(),
        )])),
    };
    assert_eq!(params.model.as_deref(), Some("sora-2"));
    assert_eq!(params.resolution.as_deref(), Some("1280x720"));
    assert_eq!(params.seconds.as_deref(), Some("8"));
    assert_eq!(params.size.as_deref(), Some("1280x720"));
    assert_eq!(params.user.as_deref(), Some("user-1"));
    assert!(params.characters.is_some());
    assert!(params.extra_headers.is_some());
    assert!(params.extra_body.is_some());
    assert!(params.parameters.is_some());
    assert!(params.image.is_some());
    assert!(params.input_reference.is_some());
}

#[test]
fn optional_params_accept_absent_values() {
    let params = VideoCreateOptionalRequestParams {
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
    };
    assert_eq!(params.model, None);
    assert_eq!(params.resolution, None);
}
