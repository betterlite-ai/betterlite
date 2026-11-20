use std::time::Duration;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use reqwest::Method;
use secrecy::SecretString;

use super::support::{
    CHARACTER_BODY, Fixture, Seen, create_call, extra_headers, params, secret, video,
};
use crate::api::video_character_transport::clients::{CharacterBody, CharacterRequest};

const BASE: &str = "https://api.example/v1";

#[test]
fn creation_posts_the_transforms_multipart_parts() {
    let mut fixture = Fixture::new();
    fixture.config.environment.insert(
        "authorization".to_owned(),
        SecretString::from("Bearer sk-provider"),
    );
    let litellm = params(&[("api_base", BASE)]);
    let clip = video();
    let call = create_call("Wren", &clip, &litellm);

    let object = fixture.sync_create(&call, None).unwrap();

    assert_eq!(object, fixture.config.object);
    assert_eq!(
        fixture.events.recorded(),
        [
            "select-sync",
            "validate",
            "url",
            "transform-create",
            "log",
            "send",
            "transform-create-response",
        ]
    );
    let requests = fixture.wire.requests();
    assert_created_multipart(&requests[0]);
}

fn assert_created_multipart(request: &CharacterRequest) {
    assert_eq!(request.method, Method::POST);
    assert_eq!(request.url.as_str(), "https://api.example/v1/characters");
    assert_eq!(request.timeout, None);
    assert_eq!(
        request
            .headers
            .get("authorization")
            .unwrap()
            .to_str()
            .unwrap(),
        "Bearer sk-provider"
    );
    let CharacterBody::Multipart { content_type, data } = &request.body else {
        panic!("a creation sends multipart form data");
    };
    let boundary = content_type
        .strip_prefix("multipart/form-data; boundary=")
        .expect("the content type names its boundary");
    let expected = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"name\"\r\n\r\nWren\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"video\"; filename=\"clip.mp4\"\r\nContent-Type: video/mp4\r\n\r\nvideo-bytes\r\n--{boundary}--\r\n"
    );
    assert_eq!(data.as_slice(), expected.as_bytes());
}

#[test]
fn creation_forwards_the_calls_timeout() {
    let mut fixture = Fixture::new();
    let litellm = params(&[("api_base", BASE)]);
    let clip = video();
    let mut call = create_call("Wren", &clip, &litellm);
    call.timeout = Some(Duration::from_millis(1500));

    fixture.sync_create(&call, None).unwrap();

    assert_eq!(
        fixture.wire.requests()[0].timeout,
        Some(Duration::from_millis(1500))
    );
}

#[test]
fn creation_sends_the_name_video_and_base_to_the_transform() {
    let mut fixture = Fixture::new();
    fixture
        .config
        .environment
        .insert("x-provider".to_owned(), secret("acme"));
    let litellm = params(&[("api_base", BASE)]);
    let clip = video();
    let call = create_call("Wren", &clip, &litellm);

    fixture.sync_create(&call, None).unwrap();

    assert_eq!(
        fixture.config.seen(),
        [
            Seen::Environment {
                api_key: None,
                model: String::new(),
                headers: Vec::new(),
            },
            Seen::Url {
                model: String::new(),
                api_base: Some(String::from(BASE)),
            },
            Seen::CreateRequest {
                name: String::from("Wren"),
                video: b"video-bytes".to_vec(),
                api_base: String::from(BASE),
                headers: vec![(String::from("x-provider"), String::from("acme"))],
            },
            Seen::Reply {
                status: 200,
                body: CHARACTER_BODY.as_bytes().to_vec(),
            },
        ]
    );
}

#[test]
fn creation_passes_the_callers_headers_and_key_to_validation() {
    let mut fixture = Fixture::new();
    let litellm = params(&[("api_base", BASE)]);
    let caller_headers = extra_headers(&[("x-caller", "yes")]);
    let key = secret("sk-caller");
    let clip = video();
    let mut call = create_call("Wren", &clip, &litellm);
    call.extra_headers = Some(&caller_headers);
    call.api_key = Some(&key);

    fixture.sync_create(&call, None).unwrap();

    let seen = fixture.config.seen();
    assert_eq!(
        seen[0],
        Seen::Environment {
            api_key: Some(String::from("sk-caller")),
            model: String::new(),
            headers: vec![(String::from("x-caller"), String::from("yes"))],
        }
    );
    let Seen::CreateRequest { headers, .. } = &seen[2] else {
        panic!("the create transform records its headers");
    };
    assert_eq!(
        headers.as_slice(),
        &[(String::from("x-caller"), String::from("yes"))]
    );
    assert_eq!(
        fixture.wire.requests()[0]
            .headers
            .get("x-caller")
            .unwrap()
            .to_str()
            .unwrap(),
        "yes"
    );
}

#[test]
fn creation_lets_the_callers_header_win_over_the_providers() {
    let mut fixture = Fixture::new();
    fixture
        .config
        .environment
        .insert("x-scope".to_owned(), secret("provider"));
    let litellm = params(&[("api_base", BASE)]);
    let caller_headers = extra_headers(&[("x-scope", "caller")]);
    let clip = video();
    let mut call = create_call("Wren", &clip, &litellm);
    call.extra_headers = Some(&caller_headers);

    fixture.sync_create(&call, None).unwrap();

    assert_eq!(
        fixture.wire.requests()[0]
            .headers
            .get("x-scope")
            .unwrap()
            .to_str()
            .unwrap(),
        "caller"
    );
}

#[test]
fn creation_falls_back_to_the_params_api_key() {
    let mut fixture = Fixture::new();
    let litellm = params(&[("api_base", BASE), ("api_key", "sk-params")]);
    let clip = video();
    let call = create_call("Wren", &clip, &litellm);

    fixture.sync_create(&call, None).unwrap();

    assert_eq!(
        fixture.config.seen()[0],
        Seen::Environment {
            api_key: Some(String::from("sk-params")),
            model: String::new(),
            headers: Vec::new(),
        }
    );
}

#[test]
fn creation_logs_the_name_as_the_pre_call_input_with_a_name_body() {
    let mut fixture = Fixture::new();
    let litellm = params(&[("api_base", BASE)]);
    let clip = video();
    let call = create_call("Wren", &clip, &litellm);

    fixture.sync_create(&call, None).unwrap();

    assert_eq!(
        fixture.state.model_call_details.get("input"),
        Some(&Json::Text(String::from("Wren")))
    );
    let Some(Json::Object(details)) = fixture.state.model_call_details.get("additional_args")
    else {
        panic!("the pre-call records its arguments");
    };
    assert_eq!(
        details.get("complete_input_dict"),
        Some(&Json::Object(IndexMap::from([(
            String::from("name"),
            Json::Text(String::from("Wren")),
        )]))),
    );
    assert_eq!(
        details.get("api_base"),
        Some(&Json::Text(String::from(
            "https://api.example/v1/characters"
        )))
    );
}
