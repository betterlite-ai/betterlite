#![cfg(test)]

use betterlite_models::decisions::Json;

use crate::api::video_records::dict_access::DictAccess;

use super::fixture::{assert_unknown_field, full_video, video};

#[test]
fn getitem_returns_the_declared_field() {
    assert_eq!(
        full_video().getitem("id"),
        Ok(Json::Text("video_123".to_owned()))
    );
}

#[test]
fn getitem_returns_null_for_an_unset_optional_field() {
    assert_eq!(video("video_123").getitem("model"), Ok(Json::Null));
}

#[test]
fn getitem_returns_the_hidden_params() {
    let video = full_video();
    assert_eq!(
        video.getitem("_hidden_params"),
        Ok(Json::Object(super::fixture::object(&[(
            "routing",
            Json::Text("openai".to_owned())
        )])))
    );
}

#[test]
fn getitem_refuses_an_unknown_field() {
    let error = full_video().getitem("duration").unwrap_err();
    assert_unknown_field(error, "VideoObject", "duration");
}
