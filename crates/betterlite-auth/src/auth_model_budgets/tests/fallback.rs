use crate::auth_model_budgets::fallback::{KeyBudgetError, check_key_budget_fallback};
use crate::auth_model_budgets::tests::fixtures::{
    Access, RecordingRewrite, ScriptedKeyLimiter, exceeded, names, other_error, text, token,
};
use crate::entity_model_access::error::EntityModelAccessError;

#[tokio::test]
async fn only_budget_errors_attempt_fallback() {
    let access = Access::default();
    let context = access.context();
    let key = token(&[("models", names(&["gpt-4"]))]);
    // a false return is neither a denial nor an error, so nothing is recorded and nothing is rewritten
    let limiter = ScriptedKeyLimiter::default();
    let mut request = RecordingRewrite::default();
    check_key_budget_fallback(
        &limiter,
        &key,
        "gpt-4",
        Some("gpt-4"),
        &mut request,
        &context,
        None,
    )
    .await
    .unwrap();
    assert_eq!(*limiter.key_calls.lock().unwrap(), vec!["gpt-4".to_owned()]);
    assert!(limiter.fallback_calls.lock().unwrap().is_empty());
    assert!(request.calls().is_empty());
    // a limiter failure that is not a budget error escapes untouched
    *limiter.failure.lock().unwrap() = Some(other_error());
    let error = check_key_budget_fallback(
        &limiter,
        &key,
        "gpt-4",
        Some("gpt-4"),
        &mut request,
        &context,
        None,
    )
    .await
    .unwrap_err();
    let KeyBudgetError::Limiter(inner) = error else {
        panic!("a non-budget limiter failure is not a budget error");
    };
    assert_eq!(*inner, other_error());
    assert!(limiter.fallback_calls.lock().unwrap().is_empty());
    assert!(request.calls().is_empty());
}

#[tokio::test]
async fn nested_model_cannot_rewrite() {
    let access = Access::default();
    let context = access.context();
    let limiter = ScriptedKeyLimiter::new(&["gpt-4"], None).with_fallback("gpt-3.5");
    let key = token(&[("models", names(&["gpt-4", "gpt-3.5"]))]);
    let mut request = RecordingRewrite::default();
    let error = check_key_budget_fallback(
        &limiter,
        &key,
        "gpt-4",
        Some("nested-model"),
        &mut request,
        &context,
        None,
    )
    .await
    .unwrap_err();
    let KeyBudgetError::Budget(details) = error else {
        panic!("a nested model keeps the budget error");
    };
    assert_eq!(*details, exceeded());
    assert!(limiter.fallback_calls.lock().unwrap().is_empty());
    assert!(request.calls().is_empty());
    // without a request model the top-level name cannot be proven either
    let error =
        check_key_budget_fallback(&limiter, &key, "gpt-4", None, &mut request, &context, None)
            .await
            .unwrap_err();
    assert!(matches!(error, KeyBudgetError::Budget(_)));
    assert!(limiter.fallback_calls.lock().unwrap().is_empty());
    assert!(request.calls().is_empty());
}

#[tokio::test]
async fn absent_fallback_preserves_error() {
    let access = Access::default();
    let context = access.context();
    let limiter = ScriptedKeyLimiter::new(&["gpt-4"], None);
    let key = token(&[("models", names(&["gpt-4"]))]);
    let mut request = RecordingRewrite::default();
    let error = check_key_budget_fallback(
        &limiter,
        &key,
        "gpt-4",
        Some("gpt-4"),
        &mut request,
        &context,
        None,
    )
    .await
    .unwrap_err();
    let KeyBudgetError::Budget(details) = error else {
        panic!("no fallback keeps the budget error");
    };
    assert_eq!(*details, exceeded());
    // the limiter was asked once for a fallback and had none to offer
    assert_eq!(
        *limiter.fallback_calls.lock().unwrap(),
        vec!["gpt-4".to_owned()]
    );
    assert_eq!(limiter.key_calls.lock().unwrap().len(), 1);
    assert!(request.calls().is_empty());
}

#[tokio::test]
async fn fallback_key_then_team_auth() {
    let access = Access::default();
    let context = access.context();
    let limiter = ScriptedKeyLimiter::new(&["gpt-4"], None).with_fallback("gpt-3.5");
    let key = token(&[
        ("models", names(&["gpt-4", "gpt-3.5"])),
        ("team_id", text("t1")),
        ("team_models", names(&["gpt-3.5"])),
    ]);
    let mut request = RecordingRewrite::default();
    check_key_budget_fallback(
        &limiter,
        &key,
        "gpt-4",
        Some("gpt-4"),
        &mut request,
        &context,
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        request.calls(),
        vec![
            "request_data",
            "parsed_body",
            "json_cache",
            "serialized_body",
            "path_params"
        ]
    );
    // the key's own grant decides first, so a team that would allow the fallback never sees it
    let key = token(&[
        ("models", names(&["gpt-4"])),
        ("team_id", text("t1")),
        ("team_models", names(&["gpt-3.5"])),
    ]);
    let mut request = RecordingRewrite::default();
    let error = check_key_budget_fallback(
        &limiter,
        &key,
        "gpt-4",
        Some("gpt-4"),
        &mut request,
        &context,
        None,
    )
    .await
    .unwrap_err();
    assert!(matches!(error, KeyBudgetError::Budget(_)));
    assert!(request.calls().is_empty());
    // a team grant that denies the fallback keeps the budget error too
    let key = token(&[
        ("models", names(&["gpt-4", "gpt-3.5"])),
        ("team_id", text("t1")),
        ("team_models", names(&["gpt-4"])),
    ]);
    let error = check_key_budget_fallback(
        &limiter,
        &key,
        "gpt-4",
        Some("gpt-4"),
        &mut request,
        &context,
        None,
    )
    .await
    .unwrap_err();
    assert!(matches!(error, KeyBudgetError::Budget(_)));
    assert!(request.calls().is_empty());
}

#[tokio::test]
async fn fallback_proxy_exception_preserves_budget_error() {
    let access = Access::default();
    let context = access.context();
    let limiter = ScriptedKeyLimiter::new(&["gpt-4"], None).with_fallback("gpt-3.5");
    let key = token(&[("models", names(&["gpt-4"]))]);
    let mut request = RecordingRewrite::default();
    let error = check_key_budget_fallback(
        &limiter,
        &key,
        "gpt-4",
        Some("gpt-4"),
        &mut request,
        &context,
        None,
    )
    .await
    .unwrap_err();
    match error {
        KeyBudgetError::Budget(ref details) => assert_eq!(**details, exceeded()),
        other @ (KeyBudgetError::Limiter(_)
        | KeyBudgetError::Access(_)
        | KeyBudgetError::Rewrite(_)) => {
            panic!("a model-access denial re-raises the budget error: {other:?}");
        }
    }
    assert_eq!(*limiter.key_calls.lock().unwrap(), vec!["gpt-4".to_owned()]);
    assert_eq!(
        *limiter.fallback_calls.lock().unwrap(),
        vec!["gpt-4".to_owned()]
    );
    assert!(request.calls().is_empty());
}

#[tokio::test]
async fn fallback_other_exception_propagates() {
    let access = Access::with_depth(0);
    let context = access.context();
    let limiter = ScriptedKeyLimiter::new(&["gpt-4"], None).with_fallback("gpt-3.5");
    let key = token(&[("models", names(&["gpt-4", "gpt-3.5"]))]);
    let mut request = RecordingRewrite::default();
    let error = check_key_budget_fallback(
        &limiter,
        &key,
        "gpt-4",
        Some("gpt-4"),
        &mut request,
        &context,
        None,
    )
    .await
    .unwrap_err();
    let KeyBudgetError::Access(EntityModelAccessError::RecurseDepth { model }) = error else {
        panic!("a non-denial access failure escapes as itself");
    };
    assert_eq!(model, "gpt-3.5");
    assert!(request.calls().is_empty());
}

#[tokio::test]
async fn fallback_rewrites_all_body_views_then_existing_path() {
    let access = Access::default();
    let context = access.context();
    let limiter = ScriptedKeyLimiter::new(&["gpt-4"], None).with_fallback("gpt-3.5");
    let key = token(&[("models", names(&["gpt-4", "gpt-3.5"]))]);
    let mut request = RecordingRewrite::default();
    check_key_budget_fallback(
        &limiter,
        &key,
        "gpt-4",
        Some("gpt-4"),
        &mut request,
        &context,
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        request.calls(),
        vec![
            "request_data",
            "parsed_body",
            "json_cache",
            "serialized_body",
            "path_params"
        ]
    );
    assert_eq!(
        *request.request_model.lock().unwrap(),
        Some("gpt-3.5".to_owned())
    );
    assert_eq!(
        *request.parsed_body.lock().unwrap(),
        Some("gpt-3.5".to_owned())
    );
    assert_eq!(
        *request.json_cache.lock().unwrap(),
        Some("gpt-3.5".to_owned())
    );
    assert_eq!(
        *request.serialized_body.lock().unwrap(),
        Some("gpt-3.5".to_owned())
    );
    assert_eq!(
        *request.path_model.lock().unwrap(),
        Some("gpt-3.5".to_owned())
    );
    // a route with no model path parameter keeps its path while the body views change
    let mut request = RecordingRewrite::default().without_path();
    check_key_budget_fallback(
        &limiter,
        &key,
        "gpt-4",
        Some("gpt-4"),
        &mut request,
        &context,
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        *request.path_model.lock().unwrap(),
        Some("gpt-4".to_owned())
    );
    assert_eq!(
        *request.request_model.lock().unwrap(),
        Some("gpt-3.5".to_owned())
    );
}

#[tokio::test]
async fn serialization_failure_keeps_prior_mutations() {
    let access = Access::default();
    let context = access.context();
    let limiter = ScriptedKeyLimiter::new(&["gpt-4"], None).with_fallback("gpt-3.5");
    let key = token(&[("models", names(&["gpt-4", "gpt-3.5"]))]);
    let mut request = RecordingRewrite::default().failing_serialization();
    let error = check_key_budget_fallback(
        &limiter,
        &key,
        "gpt-4",
        Some("gpt-4"),
        &mut request,
        &context,
        None,
    )
    .await
    .unwrap_err();
    assert!(matches!(error, KeyBudgetError::Rewrite(_)));
    assert_eq!(
        request.calls(),
        vec![
            "request_data",
            "parsed_body",
            "json_cache",
            "serialized_body"
        ]
    );
    assert_eq!(
        *request.request_model.lock().unwrap(),
        Some("gpt-3.5".to_owned())
    );
    assert_eq!(
        *request.parsed_body.lock().unwrap(),
        Some("gpt-3.5".to_owned())
    );
    assert_eq!(
        *request.json_cache.lock().unwrap(),
        Some("gpt-3.5".to_owned())
    );
    assert_eq!(*request.serialized_body.lock().unwrap(), None);
    assert_eq!(
        *request.path_model.lock().unwrap(),
        Some("gpt-4".to_owned())
    );
}
