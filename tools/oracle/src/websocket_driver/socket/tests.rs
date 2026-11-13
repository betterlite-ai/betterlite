#![cfg(test)]

use super::config;
use crate::websocket_oracle::{OracleLimits, RealtimeSettings};

#[test]
fn config_prefers_the_tighter_cap() {
    let limits = OracleLimits::default();
    let tight_settings = RealtimeSettings {
        credential_timeout_seconds: 20.0,
        max_message_size_bytes: Some(10),
    };
    let capped_by_settings = config(&limits, &tight_settings);
    assert_eq!(capped_by_settings.max_message_size, Some(10));
    assert_eq!(capped_by_settings.max_frame_size, Some(10));

    let loose_settings = RealtimeSettings {
        credential_timeout_seconds: 20.0,
        max_message_size_bytes: None,
    };
    let tight_limits = OracleLimits {
        max_frames: limits.max_frames,
        max_frame_bytes: 5,
        max_capture_bytes: limits.max_capture_bytes,
        max_execution_seconds: limits.max_execution_seconds,
    };
    let capped_by_limits = config(&tight_limits, &loose_settings);
    assert_eq!(capped_by_limits.max_message_size, Some(5));
}
