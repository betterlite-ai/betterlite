use crate::api::file_credentials::routing::{
    AuthorizedCredentialsError, RoutingOutcome, RoutingRequest, authorized_credentials,
    handle_model_based_routing,
};
use crate::api::file_routing_ids::RoutingIdType;
use crate::api::file_routing_ids::routing::encode_file_id_with_model;
use crate::api::model_credentials::{CredentialLookupError, ROUTER_MISSING_DETAIL};

use super::support::{
    Catalog, Denied, GrantAsked, Grants, Log, LookupCall, credentials, hints, locked, lookup_calls,
    object, principal,
};

#[tokio::test]
async fn routing_prefers_the_embedded_model_and_answers_the_original_id() {
    let events = Log::default();
    let mut catalog = Catalog::new(&events);
    catalog.credentials = credentials(&[(
        "model_id",
        r#"{"api_key":"ka","custom_llm_provider":"openai"}"#,
    )]);
    let grants = Grants::new(&events);
    let encoded = encode_file_id_with_model("file-raw", "model_id", RoutingIdType::File);
    let identity = principal(&[], &[], None);
    let request_hints = hints(Some("model_body"), None, None);
    let request = RoutingRequest {
        file_id: &encoded,
        hints: &request_hints,
        check_file_id_encoding: true,
    };
    let outcome = handle_model_based_routing(Some(&catalog), &grants, &request, &identity)
        .await
        .unwrap();
    assert!(outcome.use_model_routing);
    assert_eq!(outcome.model.as_deref(), Some("model_id"));
    assert_eq!(outcome.original_file_id.as_deref(), Some("file-raw"));
    assert_eq!(
        outcome.credentials,
        Some(object(r#"{"api_key":"ka","custom_llm_provider":"openai"}"#))
    );
    assert_eq!(events.entries(), ["grant model_id", "lookup model_id"]);
}

#[tokio::test]
async fn routing_without_the_id_check_takes_the_body_model() {
    let events = Log::default();
    let mut catalog = Catalog::new(&events);
    catalog.credentials = credentials(&[(
        "model_body",
        r#"{"api_key":"kb","custom_llm_provider":"openai"}"#,
    )]);
    let grants = Grants::new(&events);
    let encoded = encode_file_id_with_model("file-raw", "model_id", RoutingIdType::File);
    let identity = principal(&[], &[], None);
    let request_hints = hints(Some("model_body"), None, None);
    let request = RoutingRequest {
        file_id: &encoded,
        hints: &request_hints,
        check_file_id_encoding: false,
    };
    let outcome = handle_model_based_routing(Some(&catalog), &grants, &request, &identity)
        .await
        .unwrap();
    assert!(outcome.use_model_routing);
    assert_eq!(outcome.model.as_deref(), Some("model_body"));
    assert_eq!(outcome.original_file_id, None);
    assert_eq!(events.entries(), ["grant model_body", "lookup model_body"]);
}

#[tokio::test]
async fn routing_without_a_model_asks_nothing() {
    let events = Log::default();
    let catalog = Catalog::new(&events);
    let grants = Grants::new(&events);
    let identity = principal(&[], &[], None);
    let request_hints = hints(Some(""), None, None);
    let request = RoutingRequest {
        file_id: "file-abc",
        hints: &request_hints,
        check_file_id_encoding: true,
    };
    let outcome = handle_model_based_routing(Some(&catalog), &grants, &request, &identity)
        .await
        .unwrap();
    assert_eq!(outcome, RoutingOutcome::default());
    assert!(events.entries().is_empty());
    assert!(lookup_calls(&catalog).is_empty());
}

#[tokio::test]
async fn authorize_before_lookup_records_the_grant_then_the_lookup() {
    let events = Log::default();
    let mut catalog = Catalog::new(&events);
    catalog.credentials =
        credentials(&[("m", r#"{"api_key":"km","custom_llm_provider":"openai"}"#)]);
    let grants = Grants::new(&events);
    let identity = principal(&[], &[], None);
    let found = authorized_credentials(Some(&catalog), &grants, "m", &identity, "file operation")
        .await
        .unwrap();
    assert_eq!(
        found,
        object(r#"{"api_key":"km","custom_llm_provider":"openai"}"#)
    );
    assert_eq!(events.entries(), ["grant m", "lookup m"]);
    assert_eq!(
        locked(&grants.asked).clone(),
        [GrantAsked {
            model: "m".to_owned(),
            llm_model_list: None,
            subject: "u".to_owned(),
            router_present: true,
        }]
    );
    assert_eq!(
        lookup_calls(&catalog),
        [LookupCall {
            model_id: "m".to_owned(),
            team_id: None,
        }]
    );
}

#[tokio::test]
async fn authorize_before_lookup_denial_preserves_the_exception_and_reads_no_credentials() {
    let events = Log::default();
    let mut catalog = Catalog::new(&events);
    catalog.credentials = credentials(&[("m", r#"{"api_key":"km"}"#)]);
    let denial = Denied {
        status: 403,
        error_type: "team_model_access_denied".to_owned(),
        param: Some("m".to_owned()),
        code: Some("key_model_access_denied".to_owned()),
        detail: "not allowed to call m".to_owned(),
    };
    let mut grants = Grants::new(&events);
    grants.denial = Some(denial.clone());
    let identity = principal(&[], &[], None);
    let failure = authorized_credentials(Some(&catalog), &grants, "m", &identity, "file operation")
        .await
        .unwrap_err();
    let AuthorizedCredentialsError::Unauthorized(raised) = failure else {
        panic!("a denial comes back as the grant's own error");
    };
    assert_eq!(raised, denial);
    assert_eq!(raised.status, 403);
    assert_eq!(raised.error_type, "team_model_access_denied");
    assert_eq!(raised.param.as_deref(), Some("m"));
    assert_eq!(raised.code.as_deref(), Some("key_model_access_denied"));
    assert_eq!(events.entries(), ["grant m"]);
    assert!(lookup_calls(&catalog).is_empty());
}

#[tokio::test]
async fn authorize_before_lookup_router_missing_comes_back_unchanged() {
    let events = Log::default();
    let grants = Grants::new(&events);
    let identity = principal(&[], &[], None);
    let failure =
        authorized_credentials(None::<&Catalog>, &grants, "m", &identity, "file operation")
            .await
            .unwrap_err();
    let AuthorizedCredentialsError::Credentials(raised) = failure else {
        panic!("an allowed grant falls through to the credential lookup");
    };
    assert!(matches!(&raised, CredentialLookupError::RouterMissing));
    assert_eq!(raised.status(), Some(500));
    assert_eq!(raised.detail_error(), Some(ROUTER_MISSING_DETAIL));
}

#[tokio::test]
async fn authorize_before_lookup_missing_model_names_the_operation_context() {
    let events = Log::default();
    let catalog = Catalog::new(&events);
    let grants = Grants::new(&events);
    let identity = principal(&[], &[], None);
    let failure = authorized_credentials(Some(&catalog), &grants, "m", &identity, "file operation")
        .await
        .unwrap_err();
    let AuthorizedCredentialsError::Credentials(raised) = failure else {
        panic!("an allowed grant falls through to the credential lookup");
    };
    assert_eq!(raised.status(), Some(400));
    let detail = raised.detail_error().unwrap();
    assert!(
        detail.starts_with("file operation: Invalid model name passed in model=m."),
        "{detail}"
    );
}

#[tokio::test]
async fn authorize_before_lookup_embedded_id_uses_the_created_with_model_context() {
    let events = Log::default();
    let catalog = Catalog::new(&events);
    let grants = Grants::new(&events);
    let identity = principal(&[], &[], None);
    let encoded = encode_file_id_with_model("file-raw", "model_id", RoutingIdType::File);
    let request_hints = hints(None, None, None);
    let request = RoutingRequest {
        file_id: &encoded,
        hints: &request_hints,
        check_file_id_encoding: true,
    };
    let failure = handle_model_based_routing(Some(&catalog), &grants, &request, &identity)
        .await
        .unwrap_err();
    let AuthorizedCredentialsError::Credentials(raised) = failure else {
        panic!("an allowed grant falls through to the credential lookup");
    };
    assert_eq!(raised.status(), Some(400));
    let detail = raised.detail_error().unwrap();
    assert!(
        detail.starts_with(
            "file operation (file created with model): Invalid model name passed in model=model_id."
        ),
        "{detail}"
    );
}
