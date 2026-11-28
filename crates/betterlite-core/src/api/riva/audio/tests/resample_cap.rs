use super::super::resample::check_output_size;
use super::{
    PROBE, PYTHON, RivaAudioError, decode, install, install_scipy, oracle_host, probe_events,
    reset_fakes, soxr_source, wav_bytes,
};

// 8,388,608 samples at 8000 Hz become exactly 16,777,216 output samples, the 64 MiB cap at 4 bytes each
#[test]
fn accepts_the_largest_output_at_the_audio_cap() {
    check_output_size(0x0080_0000, 8_000).unwrap();
}

#[test]
fn rejects_one_sample_past_the_largest_output() {
    let error = check_output_size(0x0080_0001, 8_000).unwrap_err();
    assert!(matches!(error, RivaAudioError::TooLarge));
}

#[test]
fn rejects_a_sample_product_that_overflows_u64() {
    let error = check_output_size(i64::MAX, 8_000).unwrap_err();
    assert!(matches!(error, RivaAudioError::TooLarge));
}

#[test]
fn rejects_a_zero_source_rate() {
    let error = check_output_size(1_000, 0).unwrap_err();
    assert!(matches!(error, RivaAudioError::TooLarge));
}

#[test]
fn rejects_a_negative_sample_count() {
    let error = check_output_size(-1, 8_000).unwrap_err();
    assert!(matches!(error, RivaAudioError::TooLarge));
}

#[test]
fn rejects_a_negative_source_rate() {
    let error = check_output_size(1_000, -8_000).unwrap_err();
    assert!(matches!(error, RivaAudioError::TooLarge));
}

#[test]
fn rejects_a_low_rate_file_that_would_resample_past_the_cap() {
    let _guard = PYTHON.blocking_lock();
    let Some(host) = oracle_host() else {
        return;
    };
    reset_fakes(&host);
    host.attach(|py| {
        install(py, "riva_audio_probe", PROBE);
        install(py, "soxr", &soxr_source(false));
        install_scipy(py);
    });
    let silent = vec![0_i16; 110_000];
    let wav = wav_bytes(&silent, 1, 100);
    let error = decode(&host, &wav).unwrap_err();
    assert!(matches!(error, RivaAudioError::TooLarge));
    let response = error.response();
    assert_eq!(response.status_code, 413);
    assert_eq!(response.error_type, "invalid_request_error");
    assert_eq!(response.param, Some("file"));
    assert_eq!(response.code, Some("audio_too_large"));
    assert!(probe_events(&host).is_empty());
}
