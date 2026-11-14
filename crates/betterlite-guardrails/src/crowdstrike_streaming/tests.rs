#![cfg(test)]

use betterlite_config::sections::guardrail_params::params::LitellmParams;

use super::{DEFAULT_SAMPLING_RATE, StreamingParams, StreamingSettings, read_streaming_params};
use crate::crowdstrike_aidr::error::CrowdStrikeError;

fn params(text: &str) -> LitellmParams {
    serde_json::from_str(text).expect("a well-formed litellm_params")
}

#[test]
fn streaming_params_from_litellm_params() {
    let configured = params(
        r#"{
            "guardrail": "crowdstrike_aidr",
            "mode": "pre_call",
            "streaming_buffer_release_on_scan": true,
            "streaming_sampling_rate": 7,
            "streaming_end_of_stream_only": true,
            "optional_params": {
                "streaming_sampling_rate": 3,
                "streaming_buffer_until_moderated": true
            }
        }"#,
    );
    let read = read_streaming_params(&configured).expect("the values are well-typed");
    assert_eq!(read.buffer_until_moderated, Some(true));
    assert_eq!(read.buffer_release_on_scan, Some(true));
    assert_eq!(read.end_of_stream_only, Some(true));
    assert_eq!(read.sampling_rate, Some(3));
    let settings = StreamingSettings::from_params(read);
    assert!(settings.buffer_until_moderated);
    assert!(settings.buffer_release_on_scan);
    assert!(settings.end_of_stream_only);
    assert_eq!(settings.sampling_rate, 3);
}

#[test]
fn streaming_params_from_litellm_params_reads_extras_without_optional_params() {
    let configured = params(
        r#"{
            "guardrail": "crowdstrike_aidr",
            "mode": "pre_call",
            "streaming_sampling_rate": 9,
            "streaming_end_of_stream_only": true
        }"#,
    );
    let read = read_streaming_params(&configured).expect("the values are well-typed");
    assert_eq!(read.sampling_rate, Some(9));
    assert_eq!(read.end_of_stream_only, Some(true));
    assert_eq!(read.buffer_until_moderated, None);
}

#[test]
fn streaming_params_from_litellm_params_lets_optional_params_win_even_when_null() {
    let configured = params(
        r#"{
            "guardrail": "crowdstrike_aidr",
            "mode": "pre_call",
            "streaming_sampling_rate": 9,
            "optional_params": {"streaming_sampling_rate": null}
        }"#,
    );
    let read = read_streaming_params(&configured).expect("the values are well-typed");
    assert_eq!(read.sampling_rate, None);
}

#[test]
fn streaming_params_from_litellm_params_rejects_wrong_types() {
    let rate_too_low = params(
        r#"{
            "guardrail": "crowdstrike_aidr",
            "mode": "pre_call",
            "streaming_sampling_rate": 0
        }"#,
    );
    assert!(matches!(
        read_streaming_params(&rate_too_low),
        Err(CrowdStrikeError::InvalidParam {
            name: "streaming_sampling_rate"
        })
    ));
    let negative = params(
        r#"{
            "guardrail": "crowdstrike_aidr",
            "mode": "pre_call",
            "streaming_sampling_rate": -2
        }"#,
    );
    assert!(matches!(
        read_streaming_params(&negative),
        Err(CrowdStrikeError::InvalidParam {
            name: "streaming_sampling_rate"
        })
    ));
    let wrong_flag = params(
        r#"{
            "guardrail": "crowdstrike_aidr",
            "mode": "pre_call",
            "streaming_end_of_stream_only": "yes"
        }"#,
    );
    assert!(matches!(
        read_streaming_params(&wrong_flag),
        Err(CrowdStrikeError::InvalidParam {
            name: "streaming_end_of_stream_only"
        })
    ));
    let wrong_optional = params(
        r#"{
            "guardrail": "crowdstrike_aidr",
            "mode": "pre_call",
            "optional_params": 3
        }"#,
    );
    assert!(matches!(
        read_streaming_params(&wrong_optional),
        Err(CrowdStrikeError::InvalidParam {
            name: "optional_params"
        })
    ));
}

#[test]
fn _set_streaming_params_defaults() {
    assert_eq!(
        StreamingSettings::from_params(StreamingParams::default()),
        StreamingSettings {
            buffer_until_moderated: false,
            buffer_release_on_scan: false,
            end_of_stream_only: false,
            sampling_rate: DEFAULT_SAMPLING_RATE,
        }
    );
    assert_eq!(DEFAULT_SAMPLING_RATE, 5);
    let configured = StreamingSettings::from_params(StreamingParams {
        buffer_until_moderated: Some(true),
        buffer_release_on_scan: Some(false),
        end_of_stream_only: Some(true),
        sampling_rate: Some(1),
    });
    assert!(configured.buffer_until_moderated);
    assert!(!configured.buffer_release_on_scan);
    assert!(configured.end_of_stream_only);
    assert_eq!(configured.sampling_rate, 1);
}

#[test]
fn streaming_settings_default_matches_the_unset_fields() {
    assert_eq!(
        StreamingSettings::default(),
        StreamingSettings::from_params(StreamingParams::default())
    );
}
