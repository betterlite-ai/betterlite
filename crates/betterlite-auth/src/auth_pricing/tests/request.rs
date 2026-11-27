use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use rstest::rstest;

use crate::auth_pricing::request::{
    RequestFacts, UserParamError, enforce_user_param_check, reject_clientside_tags_check,
};

use super::support::{number, object, text};

fn facts<'request>(
    method: Option<&'request str>,
    route: &'request str,
    body: &'request JsonObject,
) -> RequestFacts<'request> {
    RequestFacts {
        method,
        route,
        body,
    }
}

#[test]
fn post_user_presence() {
    let empty = object(&[]);
    let post = facts(Some("POST"), "/v1/chat/completions", &empty);
    assert_eq!(
        enforce_user_param_check(true, &post),
        Err(UserParamError),
        "an enforced POST LLM request without a user is rejected"
    );
    assert_eq!(enforce_user_param_check(false, &post), Ok(()));
    let lower = facts(Some("post"), "/v1/chat/completions", &empty);
    assert_eq!(enforce_user_param_check(true, &lower), Err(UserParamError));
    let get = facts(Some("GET"), "/v1/chat/completions", &empty);
    assert_eq!(enforce_user_param_check(true, &get), Ok(()));
    let missing_method = facts(None, "/v1/chat/completions", &empty);
    assert_eq!(enforce_user_param_check(true, &missing_method), Ok(()));
    let present = object(&[("user", text("alice"))]);
    let with_user = facts(Some("POST"), "/v1/chat/completions", &present);
    assert_eq!(enforce_user_param_check(true, &with_user), Ok(()));
}

#[rstest]
#[case::null(Json::Null)]
#[case::empty(text(""))]
#[case::zero(number("0"))]
#[case::false_value(Json::Bool(false))]
fn null_user_present(#[case] user: Json) {
    let body = object(&[("user", user)]);
    let request = facts(Some("POST"), "/v1/chat/completions", &body);
    assert_eq!(
        enforce_user_param_check(true, &request),
        Ok(()),
        "the key's presence is what the check requires"
    );
}

#[test]
fn mcp_user_exempt() {
    let empty = object(&[]);
    let literal = facts(Some("POST"), "/mcp/tools/call", &empty);
    let pattern = facts(Some("POST"), "/mcp/anything", &empty);
    let chat = facts(Some("POST"), "/v1/chat/completions", &empty);
    assert_eq!(enforce_user_param_check(true, &literal), Ok(()));
    assert_eq!(enforce_user_param_check(true, &pattern), Ok(()));
    assert_eq!(enforce_user_param_check(true, &chat), Err(UserParamError));
}

#[test]
fn tags_presence_rejected() {
    let chat = "/v1/chat/completions";
    let empty_tags = object(&[(
        "metadata",
        Json::Object(object(&[("tags", Json::List(Vec::new()))])),
    )]);
    let null_tags = object(&[("metadata", Json::Object(object(&[("tags", Json::Null)])))]);
    let other_metadata = object(&[(
        "metadata",
        Json::Object(object(&[("other", Json::Bool(true))])),
    )]);
    let rejected = reject_clientside_tags_check(true, &facts(Some("POST"), chat, &empty_tags))
        .expect_err("client-side tags are rejected");
    assert_eq!(rejected.error_type, "bad_request_error");
    assert_eq!(rejected.param.as_deref(), Some("metadata.tags"));
    assert_eq!(rejected.code, "400");
    assert_eq!(
        rejected.message,
        "Client-side 'metadata.tags' not allowed in request. \
         'reject_clientside_metadata_tags'=true. Tags can only be set via API key metadata."
    );
    reject_clientside_tags_check(true, &facts(Some("POST"), chat, &null_tags))
        .expect_err("a null tags value is rejected on key presence");
    reject_clientside_tags_check(true, &facts(Some("POST"), chat, &other_metadata))
        .expect("metadata without a tags key is allowed");
    reject_clientside_tags_check(false, &facts(Some("POST"), chat, &empty_tags))
        .expect("the switch off allows client tags");
}

#[test]
fn non_llm_tags_ignored() {
    let tags = object(&[(
        "metadata",
        Json::Object(object(&[("tags", Json::List(vec![text("a")]))])),
    )]);
    let request = facts(Some("POST"), "/key/info", &tags);
    reject_clientside_tags_check(true, &request).expect("a non-LLM route is outside the tag check");
}

#[test]
fn litellm_metadata_unchecked() {
    let litellm_metadata = object(&[(
        "litellm_metadata",
        Json::Object(object(&[("tags", Json::List(vec![text("a")]))])),
    )]);
    let text_metadata = object(&[("metadata", text("not a dict"))]);
    let null_metadata = object(&[("metadata", Json::Null)]);
    let route = "/v1/chat/completions";
    reject_clientside_tags_check(true, &facts(Some("POST"), route, &litellm_metadata))
        .expect("litellm_metadata tags are outside the check");
    reject_clientside_tags_check(true, &facts(Some("POST"), route, &text_metadata))
        .expect("non-dict metadata is outside the check");
    reject_clientside_tags_check(true, &facts(Some("POST"), route, &null_metadata))
        .expect("null metadata is outside the check");
}
