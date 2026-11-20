#![cfg(test)]

use super::harness::{Call, harness};
use super::support::{
    API_BASE, base_params, complete_urls, extra_headers, merged_headers, number_base_params,
    operation, plain_headers, provider_headers, request_transforms, secret, sync_sends,
    validate_calls,
};
use crate::api::video_status_transport::error::VideoStatusError;
use crate::api::video_status_transport::records::{Headers, Params};

#[test]
fn request_build_validate_environment_gets_the_extra_headers() {
    let mut run = harness();
    let params = Params::new();
    let headers = plain_headers();
    let mut call = Call::new(&params);
    call.extra_headers = Some(&headers);
    run.sync(call).expect("the status call succeeds");
    let calls = validate_calls(&run.events());
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].headers, plain_headers());
    assert_eq!(calls[0].model, "");
    assert_eq!(calls[0].params, Some(Params::new()));
    assert_eq!(calls[0].api_key, None);
}

#[test]
fn request_build_validate_environment_gets_an_empty_map_without_extra_headers() {
    let mut run = harness();
    let params = Params::new();
    run.sync(Call::new(&params))
        .expect("the status call succeeds");
    assert_eq!(validate_calls(&run.events())[0].headers, Headers::new());
}

#[test]
fn request_build_validate_environment_gets_the_api_key() {
    let mut run = harness();
    let params = Params::new();
    let key = secret("sk-test");
    let mut call = Call::new(&params);
    call.api_key = Some(&key);
    run.sync(call).expect("the status call succeeds");
    assert_eq!(
        validate_calls(&run.events())[0].api_key,
        Some("sk-test".to_owned())
    );
}

#[test]
fn request_build_provider_headers_overwrite_the_callers() {
    let mut run = harness();
    run.deps.config.environment = Ok(provider_headers());
    let params = Params::new();
    let headers = extra_headers();
    let mut call = Call::new(&params);
    call.extra_headers = Some(&headers);
    run.sync(call).expect("the status call succeeds");
    let events = run.events();
    assert_eq!(validate_calls(&events)[0].headers, extra_headers());
    assert_eq!(request_transforms(&events)[0].headers, merged_headers());
    assert_eq!(sync_sends(&events)[0].1.headers, merged_headers());
}

#[test]
fn request_build_complete_url_gets_the_model_and_the_params_api_base() {
    let mut run = harness();
    let params = base_params(API_BASE);
    run.sync(Call::new(&params))
        .expect("the status call succeeds");
    let urls = complete_urls(&run.events());
    assert_eq!(urls.len(), 1);
    assert_eq!(urls[0].model, "");
    assert_eq!(urls[0].api_base, Some(API_BASE.to_owned()));
    assert_eq!(urls[0].params, base_params(API_BASE));
}

#[test]
fn request_build_complete_url_gets_none_without_an_api_base() {
    let mut run = harness();
    let params = Params::new();
    run.sync(Call::new(&params))
        .expect("the status call succeeds");
    let urls = complete_urls(&run.events());
    assert_eq!(urls[0].api_base, None);
    assert_eq!(urls[0].params, Params::new());
}

#[test]
fn request_build_refuses_an_api_base_of_another_type() {
    let mut run = harness();
    let params = number_base_params();
    let error = run
        .sync(Call::new(&params))
        .expect_err("the number is not a string");
    let VideoStatusError::FieldType { field, expected } = error else {
        panic!("a field type error, not a converted one")
    };
    assert_eq!(field, "api_base");
    assert_eq!(expected, "a string");
    let events = run.events();
    assert!(complete_urls(&events).is_empty());
    assert!(sync_sends(&events).is_empty());
}

#[test]
fn request_build_the_transform_gets_the_complete_url_and_the_send_uses_its_url() {
    let mut run = harness();
    run.deps.config.complete_url = Ok("https://complete/v1".to_owned());
    run.deps.config.transformed = Ok(("https://transformed/v1".to_owned(), operation()));
    let params = Params::new();
    let headers = plain_headers();
    let mut call = Call::new(&params);
    call.video_id = "video_2";
    call.extra_headers = Some(&headers);
    run.sync(call).expect("the status call succeeds");
    let events = run.events();
    let transforms = request_transforms(&events);
    assert_eq!(transforms[0].video_id, "video_2");
    assert_eq!(transforms[0].api_base, "https://complete/v1");
    assert_eq!(transforms[0].params, Params::new());
    assert_eq!(transforms[0].headers, plain_headers());
    assert_eq!(sync_sends(&events)[0].1.url, "https://transformed/v1");
}
