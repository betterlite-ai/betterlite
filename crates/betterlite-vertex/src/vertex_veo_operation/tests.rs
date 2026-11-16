#![cfg(test)]

use std::ffi::OsStr;
use std::fs;
use std::path::PathBuf;

use betterlite_config::vertex::video_settings;
use betterlite_prompts::arguments::json::{Json, Object};
use jiff::tz::TimeZone;
use rstest::rstest;
use serde_json::Number;

use super::content::content_response;
use super::decode::decode_vertex_veo_operation;
use crate::provider_metadata::timestamp::LocalTimeContext;

mod contents;
mod decodes;
mod errors;
mod requests;
mod responses;
mod unsupported_operations;
mod usage_defaults;

const OPERATION: &str =
    "projects/p/locations/l/publishers/google/models/veo-3.0-generate-001/operations/op-1";
const MODEL: &str = "veo-3.0-generate-001";
const PROVIDER: &str = "vertex_ai";
const VIDEO_ID: &str = "video_bGl0ZWxsbTpjdXN0b21fbGxtX3Byb3ZpZGVyOnZlcnRleF9haTttb2RlbF9pZDp2ZW8tMy4wLWdlbmVyYXRlLTAwMTt2aWRlb19pZDpwcm9qZWN0cy9wL2xvY2F0aW9ucy9sL3B1Ymxpc2hlcnMvZ29vZ2xlL21vZGVscy92ZW8tMy4wLWdlbmVyYXRlLTAwMS9vcGVyYXRpb25zL29wLTE=";

fn object<const N: usize>(fields: [(&str, Json); N]) -> Object {
    fields
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value))
        .collect()
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn number(value: i64) -> Json {
    Json::Number(Number::from(value))
}

fn request_data(parameters: Object) -> Object {
    let mut data = Object::new();
    data.insert("parameters".to_owned(), Json::Object(parameters));
    data
}

fn settings(default_video_duration_seconds: i64) -> video_settings::Settings {
    video_settings::Settings {
        default_video_duration_seconds,
    }
}

fn utc() -> LocalTimeContext {
    LocalTimeContext {
        zone: TimeZone::UTC,
    }
}

fn encoded<T>(value: &T) -> Vec<u8>
where
    T: serde::Serialize,
{
    serde_json::to_vec(value).expect("the body serializes")
}

// the seeds are named for the decoder path they exercise, so a new seed states its expectation in its name
#[rstest]
fn replays_the_vertex_veo_operation_corpus(
    #[files("fuzz/corpus/vertex_veo_operation/*")] path: PathBuf,
) {
    let bytes = fs::read(&path).expect("a readable seed");
    let name = path
        .file_name()
        .and_then(OsStr::to_str)
        .expect("a utf-8 seed name");
    let decoded = decode_vertex_veo_operation(&bytes);
    if name.starts_with("malformed") {
        assert!(decoded.is_err(), "{name}");
        return;
    }
    assert!(decoded.is_ok(), "{name}");
    let content = content_response(&bytes);
    if name.starts_with("content-video") {
        assert!(content.is_ok(), "{name}");
    } else if name.starts_with("content-") {
        assert!(content.is_err(), "{name}");
    }
}
