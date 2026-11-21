use std::collections::BTreeSet;

use betterlite_auth::auth_request_models::sources::DispatchContext;
use betterlite_core::api::request_destination::RequestData;
use betterlite_core::api::request_principal_metadata::{Metadata, PrincipalFacts};
use betterlite_models::decisions::Json;
use http::HeaderMap;
use http::header::HeaderValue;

use crate::failure_hook::lift::PASS_THROUGH_CALL_TYPE;
use crate::passthrough::PassthroughLogPayload;
use crate::passthrough::callbacks::TRUSTED_CALLBACK_VARS_FIELD;
use crate::passthrough::request_context::{
    MODEL_ACCESS_GROUP_METADATA_KEY, PassThroughContextError, PassThroughInit, PassThroughKey,
    PassThroughKwargs, PassThroughRequest, PassthroughLoggingObj, SESSION_ID_OMITTED_METADATA_KEY,
    init_pass_through_kwargs,
};

const KEY_JSON: &str = r#"{
    "api_key": "hashed-key",
    "matched_model_access_groups": ["group-a"],
    "budget_reservation": {"reservation_id": "r1"}
}"#;

const BODY_JSON: &str = r#"{
    "model": "upstream-model",
    "custom_param": {"a": 1},
    "custom_llm_provider": "upstream-provider",
    "metadata": {
        "team_tag": "t1",
        "user_api_key_hash": "spoofed",
        "tags": ["old"],
        "litellm_session_id_omitted": "client"
    },
    "litellm_metadata": {"trace_id": "abc"}
}"#;

fn principal(text: &str) -> PrincipalFacts {
    serde_json::from_str(text).unwrap()
}

fn data(text: &str) -> RequestData {
    serde_json::from_str(text).unwrap()
}

fn json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

fn object(value: &Json) -> &Metadata {
    if let Json::Object(fields) = value {
        return fields;
    }
    panic!("not an object: {value:?}");
}

fn text(value: &Json) -> &str {
    if let Json::Text(text) = value {
        return text;
    }
    panic!("not text: {value:?}");
}

fn tags(value: &Json) -> Vec<&str> {
    if let Json::List(items) = value {
        return items.iter().map(text).collect();
    }
    panic!("not a list: {value:?}");
}

fn headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("tags", HeaderValue::from_static("one, two"));
    headers.insert("x-litellm-tags", HeaderValue::from_static("three"));
    headers.insert("x-custom", HeaderValue::from_static("first"));
    headers.append("x-custom", HeaderValue::from_static("second"));
    headers
}

fn payload() -> PassthroughLogPayload {
    PassthroughLogPayload {
        url: "https://upstream.example/v1".to_owned(),
        request_method: Some("POST".to_owned()),
        request_body: None,
        response_body: None,
        cost_per_request: None,
    }
}

fn key() -> PassThroughKey {
    PassThroughKey {
        principal: principal(KEY_JSON),
        parent_otel_span: Json::Text("span-1".to_owned()),
        model_max_budget: Some(data(r#"{"max_budget": 5}"#)),
        team_model_max_budget: None,
        user_model_max_budget: None,
        end_user_model_max_budget: None,
    }
}

fn initialized(
    dispatch: &DispatchContext<'_>,
    user_api_key_dict: &PassThroughKey,
) -> (PassThroughKwargs, PassthroughLoggingObj) {
    let headers = headers();
    let model_info = data(r#"{"id": "deployment-1"}"#);
    let payload = payload();
    let mut logging_obj = PassthroughLoggingObj::default();
    let kwargs = init_pass_through_kwargs(PassThroughInit {
        request: PassThroughRequest {
            url: "https://upstream.example/v1?q=1",
            method: "POST",
            headers: &headers,
            deployment_model_info: Some(&model_info),
        },
        user_api_key_dict,
        passthrough_logging_payload: payload,
        logging_obj: &mut logging_obj,
        parsed_body: Some(data(BODY_JSON)),
        litellm_call_id: Some("call-1".to_owned()),
        dispatch,
    })
    .unwrap();
    (kwargs, logging_obj)
}

fn assert_forwarded(kwargs: &PassThroughKwargs, logging_obj: &PassthroughLoggingObj) {
    assert_eq!(kwargs.call_type, PASS_THROUGH_CALL_TYPE);
    assert_eq!(kwargs.call_type, "pass_through_endpoint");
    assert_eq!(kwargs.litellm_call_id.as_deref(), Some("call-1"));
    assert_eq!(kwargs.passthrough_logging_payload, payload());
    let details = &logging_obj.model_call_details;
    assert_eq!(
        details.get("passthrough_logging_payload"),
        Some(&payload().to_json())
    );
    let data = kwargs.to_data();
    let params = object(&data["litellm_params"]);
    assert!(params.get("litellm_metadata").is_none());
    assert_eq!(text(&params["custom_llm_provider"]), "upstream-provider");
    let proxy_request = object(&params["proxy_server_request"]);
    assert_eq!(
        text(&proxy_request["url"]),
        "https://upstream.example/v1?q=1"
    );
    assert_eq!(text(&proxy_request["method"]), "POST");
}

fn assert_metadata(kwargs: &PassThroughKwargs) {
    let data = kwargs.to_data();
    let params = object(&data["litellm_params"]);
    let metadata = object(&params["metadata"]);
    assert_eq!(text(&metadata["team_tag"]), "t1");
    assert_eq!(text(&metadata["trace_id"]), "abc");
    assert_eq!(tags(&metadata["tags"]), vec!["old", "one", "two", "three"]);
    assert!(metadata.get(SESSION_ID_OMITTED_METADATA_KEY).is_none());
    // the authenticated key wins over a client body that mirrors its fields
    assert_eq!(text(&metadata["user_api_key_hash"]), "hashed-key");
    assert_eq!(text(&metadata["user_api_key"]), "hashed-key");
    assert_eq!(text(&metadata["litellm_parent_otel_span"]), "span-1");
    assert_eq!(
        text(&object(&metadata["user_api_key_budget_reservation"])["reservation_id"]),
        "r1"
    );
    assert_eq!(
        tags(&metadata[MODEL_ACCESS_GROUP_METADATA_KEY]),
        vec!["group-a"]
    );
    assert_eq!(
        object(&metadata["model_info"])["id"],
        Json::Text("deployment-1".to_owned())
    );
    assert_eq!(
        object(&metadata["user_api_key_model_max_budget"])["max_budget"],
        json("5")
    );
    assert_eq!(metadata["user_api_key_team_model_max_budget"], Json::Null);
}

fn assert_proxy_request(kwargs: &PassThroughKwargs) {
    let data = kwargs.to_data();
    let params = object(&data["litellm_params"]);
    let proxy_request = object(&params["proxy_server_request"]);
    let body = object(&proxy_request["body"]);
    assert!(body.get("metadata").is_none());
    assert!(body.get("litellm_metadata").is_none());
    assert!(body.get("custom_llm_provider").is_none());
    assert_eq!(text(&body["model"]), "upstream-model");
    assert_eq!(body["custom_param"], json(r#"{"a": 1}"#));
    let sent_headers = object(&proxy_request["headers"]);
    assert_eq!(text(&sent_headers["x-custom"]), "first, second");
    assert_eq!(text(&sent_headers["tags"]), "one, two");
}

fn assert_no_budgets(kwargs: &PassThroughKwargs) {
    let data = kwargs.to_data();
    let params = object(&data["litellm_params"]);
    let metadata = object(&params["metadata"]);
    assert!(metadata.get("user_api_key_model_max_budget").is_none());
    assert!(metadata.get("user_api_key_team_model_max_budget").is_none());
    assert!(metadata.get("user_api_key_user_model_max_budget").is_none());
    assert!(
        metadata
            .get("user_api_key_end_user_model_max_budget")
            .is_none()
    );
    assert_eq!(text(&metadata["user_api_key"]), "hashed-key");
}

fn assert_metadata_type_error(user_api_key_dict: &PassThroughKey) {
    let headers = headers();
    let paths = BTreeSet::new();
    let dispatch = DispatchContext {
        endpoint_marker: None,
        path_params: &paths,
    };
    let mut logging_obj = PassthroughLoggingObj::default();
    let error = init_pass_through_kwargs(PassThroughInit {
        request: PassThroughRequest {
            url: "https://upstream.example/v1",
            method: "POST",
            headers: &headers,
            deployment_model_info: None,
        },
        user_api_key_dict,
        passthrough_logging_payload: payload(),
        logging_obj: &mut logging_obj,
        parsed_body: Some(data(r#"{"metadata": "not-an-object"}"#)),
        litellm_call_id: None,
        dispatch: &dispatch,
    });
    assert!(matches!(
        error,
        Err(PassThroughContextError::MetadataNotObject { field: "metadata" })
    ));
}

#[test]
fn passthrough_init_kwargs() {
    let user_api_key_dict = key();
    let paths = BTreeSet::new();
    let dispatch = DispatchContext {
        endpoint_marker: None,
        path_params: &paths,
    };
    let (kwargs, logging_obj) = initialized(&dispatch, &user_api_key_dict);
    assert_forwarded(&kwargs, &logging_obj);
    assert_metadata(&kwargs);
    assert_proxy_request(&kwargs);

    // a user-defined pass-through route skips the per-model budget counters entirely
    let dispatched = DispatchContext {
        endpoint_marker: Some(true),
        path_params: &paths,
    };
    let (user_defined, _) = initialized(&dispatched, &user_api_key_dict);
    assert_no_budgets(&user_defined);

    assert_metadata_type_error(&user_api_key_dict);
}

#[test]
fn passthrough_request_context() {
    assert_eq!(
        SESSION_ID_OMITTED_METADATA_KEY,
        "litellm_session_id_omitted"
    );
    assert_eq!(
        MODEL_ACCESS_GROUP_METADATA_KEY,
        "user_api_key_matched_model_access_groups"
    );
    assert_eq!(TRUSTED_CALLBACK_VARS_FIELD, "litellm_trusted_callback_vars");
    assert_eq!(PASS_THROUGH_CALL_TYPE, "pass_through_endpoint");
    // the logging object carries the payload as the dict LiteLLM's logger stores
    assert_eq!(
        payload().to_json(),
        json(
            r#"{"url":"https://upstream.example/v1","request_method":"POST","request_body":null,"response_body":null,"cost_per_request":null}"#
        )
    );
}
