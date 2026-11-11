#![cfg(test)]

use betterlite_anthropic::credentials::AnthropicSettings;
use http::HeaderMap;
use rstest::rstest;

use crate::api::skills::fixtures::{
    Fixture, SKILL_ID, anthropic_settings, call, call_with, kwargs, logging, skill_request,
};
use crate::api::skills::get_delete::call::SkillsCallParams;
use crate::api::skills::get_delete::service::api_base;

#[tokio::test]
async fn get_skill_url_targets_the_skill_endpoint() {
    let fixture = Fixture::new();
    let values = kwargs();
    fixture
        .service()
        .get_skill(skill_request(&call(SKILL_ID, &values), &mut logging()))
        .await
        .expect("the external get succeeds");
    let requests = fixture.http.get_requests.lock().expect("the fake");
    assert_eq!(
        requests[0].url,
        "https://api.anthropic.com/v1/skills/skill_1"
    );
}

#[tokio::test]
async fn delete_skill_url_targets_the_skill_endpoint() {
    let fixture = Fixture::new();
    let values = kwargs();
    fixture
        .service()
        .delete_skill(skill_request(&call(SKILL_ID, &values), &mut logging()))
        .await
        .expect("the external delete succeeds");
    let requests = fixture.http.delete_requests.lock().expect("the fake");
    assert_eq!(
        requests[0].url,
        "https://api.anthropic.com/v1/skills/skill_1"
    );
}

#[tokio::test]
async fn url_construction_encodes_the_skill_id() {
    let fixture = Fixture::new();
    let values = kwargs();
    fixture
        .service()
        .get_skill(skill_request(&call("skill/1 ?x", &values), &mut logging()))
        .await
        .expect("the external get succeeds");
    let requests = fixture.http.get_requests.lock().expect("the fake");
    assert_eq!(
        requests[0].url,
        "https://api.anthropic.com/v1/skills/skill%2F1%20%3Fx"
    );
}

#[tokio::test]
async fn fallback_base_has_a_single_v1_segment() {
    let fixture = Fixture::new();
    let values = kwargs();
    fixture
        .service()
        .get_skill(skill_request(&call(SKILL_ID, &values), &mut logging()))
        .await
        .expect("the external get succeeds");
    let requests = fixture.http.get_requests.lock().expect("the fake");
    assert_eq!(
        requests[0].url,
        "https://api.anthropic.com/v1/skills/skill_1"
    );
    assert_eq!(requests[0].url.matches("/v1/").count(), 1);
}

#[tokio::test]
async fn get_skill_url_prefers_the_call_api_base() {
    let fixture = Fixture::new();
    let values = kwargs();
    let params = SkillsCallParams {
        api_key: None,
        api_base: Some("https://gateway.example".to_owned()),
        extra_headers: HeaderMap::new(),
        extra_query: None,
        timeout: None,
        custom_llm_provider: None,
    };
    let request = call_with(params, SKILL_ID, &values);
    fixture
        .service()
        .get_skill(skill_request(&request, &mut logging()))
        .await
        .expect("the external get succeeds");
    let requests = fixture.http.get_requests.lock().expect("the fake");
    assert_eq!(requests[0].url, "https://gateway.example/v1/skills/skill_1");
}

#[tokio::test]
async fn url_construction_ignores_an_empty_call_api_base() {
    let fixture = Fixture::new();
    let values = kwargs();
    let params = SkillsCallParams {
        api_key: None,
        api_base: Some(String::new()),
        extra_headers: HeaderMap::new(),
        extra_query: None,
        timeout: None,
        custom_llm_provider: None,
    };
    let request = call_with(params, SKILL_ID, &values);
    fixture
        .service()
        .get_skill(skill_request(&request, &mut logging()))
        .await
        .expect("the external get succeeds");
    let requests = fixture.http.get_requests.lock().expect("the fake");
    assert_eq!(
        requests[0].url,
        "https://api.anthropic.com/v1/skills/skill_1"
    );
}

#[rstest]
#[case::call_base(Some("https://gateway.example"), "https://gateway.example")]
#[case::empty_call_base(Some(""), "https://env-base.example")]
#[case::absent_call_base(None, "https://env-base.example")]
fn call_base_resolution_uses_only_a_non_empty_api_base(
    #[case] call_base: Option<&str>,
    #[case] expected: &str,
) {
    // a distinct settings base proves the fallback reads B567's resolved value, not a literal default
    let settings = AnthropicSettings {
        api_base: "https://env-base.example".to_owned(),
        ..anthropic_settings()
    };
    let params = SkillsCallParams {
        api_key: None,
        api_base: call_base.map(str::to_owned),
        extra_headers: HeaderMap::new(),
        extra_query: None,
        timeout: None,
        custom_llm_provider: None,
    };
    assert_eq!(api_base(&params, &settings), expected);
}
