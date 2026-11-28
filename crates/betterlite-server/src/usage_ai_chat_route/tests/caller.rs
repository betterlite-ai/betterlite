use crate::auth::auth_key_lookup::tests::caller_through_key_stage;

const TOOL_BODY: &str = r#"{"messages": [{"role": "user", "content": "How much did I spend?"}]}"#;

#[test]
fn caller_takes_an_admins_own_user_id_and_allows_null() {
    let caller = FakeCallerId::from_subject();
    let with_user =
        identity(&admin_principal("ana"), &caller).expect("an admin needs no caller guard");
    assert_eq!(with_user.user_id.as_deref(), Some("ana"));
    assert!(with_user.is_admin);
    let master =
        identity(&master_principal(), &caller).expect("the master key needs no caller guard");
    assert_eq!(master.user_id.as_deref(), Some("default_user_id"));
    assert!(master.is_admin);
    let without_user = identity(
        &principal("role-admin", None, &[Role::PlatformAdmin]),
        &caller,
    )
    .expect("an admin without a user needs no caller guard");
    assert_eq!(without_user.user_id, None);
    assert!(without_user.is_admin);
    let viewer = identity(
        &principal("viewer", Some("viewer"), &[Role::PlatformViewer]),
        &caller,
    )
    .expect("an admin viewer needs no caller guard");
    assert_eq!(viewer.user_id.as_deref(), Some("viewer"));
    assert!(viewer.is_admin);
    assert_eq!(caller.calls(), 0);
}

#[tokio::test]
async fn caller_uses_the_real_guard_id_for_a_user_bound_non_admin() {
    let model = FakeModel::new()
        .replying(usage_tool_reply())
        .replying(answer_reply("You spent nothing yet."));
    let source = FakeSource::new();
    let factory = FakeFactory::new(model.clone(), source.clone());
    let request = with_principal(post(TOOL_BODY), principal("key-1", Some("owner-1"), &[]));
    let response = send(state(factory, None, GuardCallerId), request).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(body_text(response).await.contains("\"type\": \"done\""));
    assert_eq!(source.user_ids(), vec![Some("owner-1".to_owned())]);
    assert_eq!(model.calls()[0].tools, 1);
}

#[tokio::test]
async fn caller_answers_a_user_bound_key_from_the_key_stage() {
    let model = FakeModel::new()
        .replying(usage_tool_reply())
        .replying(answer_reply("You spent nothing yet."));
    let source = FakeSource::new();
    let factory = FakeFactory::new(model.clone(), source.clone());
    let request = with_principal(
        post(TOOL_BODY),
        caller_through_key_stage(r#"{"user_id":"u1"}"#).principal,
    );
    let response = send(state(factory, None, GuardCallerId), request).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(body_text(response).await.contains("\"type\": \"done\""));
    assert_eq!(source.user_ids(), vec![Some("u1".to_owned())]);
    assert_eq!(model.calls()[0].tools, 1);
}

#[tokio::test]
async fn caller_treats_a_key_whose_user_is_master_key_as_a_non_admin() {
    let model = FakeModel::new()
        .replying(usage_tool_reply())
        .replying(answer_reply("You spent nothing yet."));
    let source = FakeSource::new();
    let factory = FakeFactory::new(model.clone(), source.clone());
    let request = with_principal(
        post(TOOL_BODY),
        caller_through_key_stage(r#"{"user_id":"master_key"}"#).principal,
    );
    let response = send(state(factory, None, GuardCallerId), request).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(body_text(response).await.contains("\"type\": \"done\""));
    assert_eq!(source.user_ids(), vec![Some("master_key".to_owned())]);
    assert_eq!(model.calls()[0].tools, 1);
}

#[tokio::test]
async fn caller_uses_the_guards_id_for_a_non_admin() {
    let model = FakeModel::new()
        .replying(usage_tool_reply())
        .replying(answer_reply("You spent nothing yet."));
    let source = FakeSource::new();
    let caller = FakeCallerId::from_subject();
    let factory = FakeFactory::new(model.clone(), source.clone());
    let request = with_principal(post(TOOL_BODY), principal("guard-id", Some("owner-1"), &[]));
    let response = send(state(factory, None, caller.clone()), request).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(body_text(response).await.contains("\"type\": \"done\""));
    assert_eq!(caller.calls(), 1);
    assert_eq!(source.user_ids(), vec![Some("guard-id".to_owned())]);
    assert_eq!(model.calls()[0].tools, 1);
}

#[rstest]
#[case::master_key(master_principal())]
#[case::platform_admin(admin_principal("ana"))]
#[case::platform_viewer(principal("viewer", Some("viewer"), &[Role::PlatformViewer]))]
#[case::key_admin_view(key_admin_principal("key-owner"))]
#[tokio::test]
async fn caller_treats_an_admin_view_as_an_admin(#[case] who: Principal) {
    let model = FakeModel::new()
        .replying(usage_tool_reply())
        .replying(answer_reply("You spent nothing yet."));
    let source = FakeSource::new();
    let caller = FakeCallerId::from_subject();
    let factory = FakeFactory::new(model.clone(), source.clone());
    let request = with_principal(post(TOOL_BODY), who);
    let response = send(state(factory, None, caller.clone()), request).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(body_text(response).await.contains("\"type\": \"done\""));
    assert_eq!(caller.calls(), 0);
    assert_eq!(source.user_ids(), vec![None]);
    assert_eq!(model.calls()[0].tools, 3);
}

#[tokio::test]
async fn caller_returns_the_guard_error_before_the_stream() {
    let model = FakeModel::new().replying(answer_reply("Hi"));
    let factory = FakeFactory::new(model.clone(), FakeSource::new());
    let request = with_principal(post(TOOL_BODY), principal("svc", None, &[]));
    let response = send(state(factory, None, GuardCallerId), request).await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        parsed(&body_text(response).await),
        parsed(
            r#"{"detail": {"error": "Service-account keys cannot query user analytics. Use a user-bound key, or call as a proxy admin."}}"#
        )
    );
    assert!(model.calls().is_empty());
}
