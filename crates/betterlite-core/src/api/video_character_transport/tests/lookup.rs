use std::time::Duration;

use betterlite_models::decisions::Json;
use reqwest::Method;

use super::support::{CHARACTER_BODY, Fixture, Seen, character_object, get_call, params};
use crate::api::video_character_transport::clients::CharacterBody;

const BASE: &str = "https://api.example/v1";
const CHARACTER_URL: &str = "https://api.example/v1/characters/char-1";

#[test]
fn lookup_sends_a_get_with_the_transforms_params() {
    let mut fixture = Fixture::new();
    fixture.config.query = vec![(String::from("expand"), String::from("1"))];
    let litellm = params(&[("api_base", BASE)]);
    let call = get_call("char-1", &litellm);

    let object = fixture.sync_get(&call, None).unwrap();

    assert_eq!(object, fixture.config.object);
    assert_eq!(
        fixture.events.recorded(),
        [
            "select-sync",
            "validate",
            "url",
            "transform-get",
            "log",
            "send",
            "transform-get-response",
        ]
    );
    let requests = fixture.wire.requests();
    let request = &requests[0];
    assert_eq!(request.method, Method::GET);
    assert_eq!(
        request.url.as_str(),
        "https://api.example/v1/characters/char-1?expand=1"
    );
    assert!(matches!(&request.body, CharacterBody::Empty));
    assert_eq!(
        fixture.config.seen()[2],
        Seen::GetRequest {
            character_id: String::from("char-1"),
            api_base: String::from(BASE),
            headers: Vec::new(),
        }
    );
}

#[test]
fn lookup_keeps_the_urls_own_query_value() {
    let mut fixture = Fixture::new();
    fixture.config.get_url = String::from("https://api.example/v1/characters/char-1?expand=url");
    fixture.config.query = vec![
        (String::from("expand"), String::from("param")),
        (String::from("view"), String::from("full")),
    ];
    let litellm = params(&[("api_base", BASE)]);
    let call = get_call("char-1", &litellm);

    fixture.sync_get(&call, None).unwrap();

    let requests = fixture.wire.requests();
    let query: Vec<(String, String)> = requests[0].url.query_pairs().into_owned().collect();
    assert_eq!(
        query,
        vec![
            (String::from("expand"), String::from("url")),
            (String::from("view"), String::from("full")),
        ]
    );
}

#[test]
fn lookup_never_forwards_the_calls_timeout() {
    let mut fixture = Fixture::new();
    let litellm = params(&[("api_base", BASE)]);
    let mut call = get_call("char-1", &litellm);
    call.timeout = Some(Duration::from_secs(2));

    fixture.sync_get(&call, None).unwrap();

    assert_eq!(fixture.wire.requests()[0].timeout, None);
}

#[test]
fn lookup_logs_the_character_id_without_a_body() {
    let mut fixture = Fixture::new();
    let litellm = params(&[("api_base", BASE)]);
    let call = get_call("char-1", &litellm);

    fixture.sync_get(&call, None).unwrap();

    assert_eq!(
        fixture.state.model_call_details.get("input"),
        Some(&Json::Text(String::from("char-1")))
    );
    let Some(Json::Object(details)) = fixture.state.model_call_details.get("additional_args")
    else {
        panic!("the pre-call records its arguments");
    };
    assert_eq!(details.get("complete_input_dict"), None);
    assert_eq!(
        details.get("api_base"),
        Some(&Json::Text(String::from(CHARACTER_URL)))
    );
}

#[test]
fn lookup_returns_the_object_the_transform_builds() {
    let mut fixture = Fixture::new();
    fixture.config.object = character_object("Ash");
    let litellm = params(&[("api_base", BASE)]);
    let call = get_call("char-1", &litellm);

    let object = fixture.sync_get(&call, None).unwrap();

    assert_eq!(object.name, "Ash");
    assert_eq!(
        fixture.config.seen().last(),
        Some(&Seen::Reply {
            status: 200,
            body: CHARACTER_BODY.as_bytes().to_vec(),
        })
    );
}
