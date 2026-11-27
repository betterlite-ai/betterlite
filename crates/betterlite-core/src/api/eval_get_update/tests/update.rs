use betterlite_openai::evals::types::UpdateEvalRequest;

use crate::api::eval_get_update::call::EvalUpdate;
use crate::api::eval_get_update::tests::support::{
    Calls, Harness, eval, internal_metadata, json, map, object, update_call,
};

#[tokio::test]
async fn update_eval_builds_the_request_from_name_and_metadata() {
    let harness = Harness::new();
    let calls = Calls::empty();
    let metadata = map(&[("team", json("\"core\""))]);
    let call = update_call(&calls, "eval_1", Some("renamed"), Some(&metadata));
    let mut state = harness.logging_state("aupdate_eval");
    let env = harness.env.pre_post();
    harness.handler.answer_update(Ok(eval("eval_1")));
    let update = EvalUpdate {
        call: &call,
        logging: &mut state,
        env: &env,
    };
    let returned = harness
        .service()
        .update_eval(update)
        .await
        .expect("the update succeeds");
    assert_eq!(returned.id, "eval_1");
    let updates = harness.handler.updates();
    assert_eq!(updates.len(), 1);
    assert_eq!(
        updates[0].body,
        object(r#"{"metadata":{"team":"core"},"name":"renamed"}"#)
    );
    assert_eq!(updates[0].url, "https://api.openai.com/v1/evals/eval_1");
    let recorded = harness.recorded();
    assert_eq!(
        recorded.updates[0].request,
        UpdateEvalRequest {
            name: Some("renamed".to_owned()),
            metadata: Some(object(r#"{"team":"core"}"#)),
        }
    );
    assert_eq!(recorded.updates[0].eval_id, "eval_1");
    assert_eq!(recorded.updates[0].api_base, "https://api.openai.com");
}

#[tokio::test]
async fn update_eval_sends_an_empty_body_when_nothing_is_given() {
    let harness = Harness::new();
    let calls = Calls::empty();
    let call = update_call(&calls, "eval_1", None, None);
    let mut state = harness.logging_state("aupdate_eval");
    let env = harness.env.pre_post();
    harness.handler.answer_update(Ok(eval("eval_1")));
    let update = EvalUpdate {
        call: &call,
        logging: &mut state,
        env: &env,
    };
    harness
        .service()
        .update_eval(update)
        .await
        .expect("the update succeeds");
    let updates = harness.handler.updates();
    assert_eq!(updates[0].body, object("{}"));
    let recorded = harness.recorded();
    assert_eq!(recorded.updates[0].request.name, None);
    assert_eq!(recorded.updates[0].request.metadata, None);
}

#[tokio::test]
async fn update_eval_filters_every_internal_metadata_key() {
    let harness = Harness::new();
    let calls = Calls::empty();
    let mut metadata = internal_metadata();
    metadata.insert("team".to_owned(), json("\"core\""));
    metadata.insert("tier".to_owned(), json("\"gold\""));
    let call = update_call(&calls, "eval_1", None, Some(&metadata));
    let mut state = harness.logging_state("aupdate_eval");
    let env = harness.env.pre_post();
    harness.handler.answer_update(Ok(eval("eval_1")));
    let update = EvalUpdate {
        call: &call,
        logging: &mut state,
        env: &env,
    };
    harness
        .service()
        .update_eval(update)
        .await
        .expect("the update succeeds");
    let updates = harness.handler.updates();
    assert_eq!(
        updates[0].body,
        object(r#"{"metadata":{"team":"core","tier":"gold"}}"#)
    );
    let recorded = harness.recorded();
    assert_eq!(
        recorded.updates[0].request.metadata,
        Some(object(r#"{"team":"core","tier":"gold"}"#))
    );
}

#[tokio::test]
async fn update_eval_omits_metadata_when_all_keys_are_internal() {
    let harness = Harness::new();
    let calls = Calls::empty();
    let metadata = internal_metadata();
    let call = update_call(&calls, "eval_1", None, Some(&metadata));
    let mut state = harness.logging_state("aupdate_eval");
    let env = harness.env.pre_post();
    harness.handler.answer_update(Ok(eval("eval_1")));
    let update = EvalUpdate {
        call: &call,
        logging: &mut state,
        env: &env,
    };
    harness
        .service()
        .update_eval(update)
        .await
        .expect("the update succeeds");
    let updates = harness.handler.updates();
    assert_eq!(updates[0].body, object("{}"));
    assert_eq!(harness.recorded().updates[0].request.metadata, None);
}

#[tokio::test]
async fn update_eval_omits_an_empty_metadata_map() {
    let harness = Harness::new();
    let calls = Calls::empty();
    let metadata = map(&[]);
    let call = update_call(&calls, "eval_1", None, Some(&metadata));
    let mut state = harness.logging_state("aupdate_eval");
    let env = harness.env.pre_post();
    harness.handler.answer_update(Ok(eval("eval_1")));
    let update = EvalUpdate {
        call: &call,
        logging: &mut state,
        env: &env,
    };
    harness
        .service()
        .update_eval(update)
        .await
        .expect("the update succeeds");
    let updates = harness.handler.updates();
    assert_eq!(updates[0].body, object("{}"));
    assert_eq!(harness.recorded().updates[0].request.metadata, None);
}

#[tokio::test]
async fn update_eval_extra_body_replaces_the_filtered_metadata() {
    let harness = Harness::new();
    let mut calls = Calls::empty();
    calls.extra_body = map(&[(
        "metadata",
        json("{\"team\":\"override\",\"tier\":\"gold\"}"),
    )]);
    let metadata = map(&[("team", json("\"core\""))]);
    let call = update_call(&calls, "eval_1", None, Some(&metadata));
    let mut state = harness.logging_state("aupdate_eval");
    let env = harness.env.pre_post();
    harness.handler.answer_update(Ok(eval("eval_1")));
    let update = EvalUpdate {
        call: &call,
        logging: &mut state,
        env: &env,
    };
    harness
        .service()
        .update_eval(update)
        .await
        .expect("the update succeeds");
    let updates = harness.handler.updates();
    assert_eq!(
        updates[0].body,
        object(r#"{"metadata":{"team":"override","tier":"gold"}}"#)
    );
    assert_eq!(
        harness.recorded().updates[0].request.metadata,
        Some(object(r#"{"team":"override","tier":"gold"}"#))
    );
}

#[tokio::test]
async fn update_eval_extra_body_restores_metadata_bypassing_the_filter() {
    let harness = Harness::new();
    let mut calls = Calls::empty();
    calls.extra_body = map(&[(
        "metadata",
        json("{\"user_api_key_hash\":\"leaked\",\"team\":\"core\"}"),
    )]);
    let metadata = internal_metadata();
    let call = update_call(&calls, "eval_1", None, Some(&metadata));
    let mut state = harness.logging_state("aupdate_eval");
    let env = harness.env.pre_post();
    harness.handler.answer_update(Ok(eval("eval_1")));
    let update = EvalUpdate {
        call: &call,
        logging: &mut state,
        env: &env,
    };
    harness
        .service()
        .update_eval(update)
        .await
        .expect("the update succeeds");
    let updates = harness.handler.updates();
    assert_eq!(
        updates[0].body,
        object(r#"{"metadata":{"user_api_key_hash":"leaked","team":"core"}}"#)
    );
}

#[tokio::test]
async fn update_eval_extra_body_overrides_the_name() {
    let harness = Harness::new();
    let mut calls = Calls::empty();
    calls.extra_body = map(&[("name", json("\"renamed\""))]);
    let call = update_call(&calls, "eval_1", Some("original"), None);
    let mut state = harness.logging_state("aupdate_eval");
    let env = harness.env.pre_post();
    harness.handler.answer_update(Ok(eval("eval_1")));
    let update = EvalUpdate {
        call: &call,
        logging: &mut state,
        env: &env,
    };
    harness
        .service()
        .update_eval(update)
        .await
        .expect("the update succeeds");
    let updates = harness.handler.updates();
    assert_eq!(updates[0].body, object(r#"{"name":"renamed"}"#));
    assert_eq!(
        harness.recorded().updates[0].request.name,
        Some("renamed".to_owned())
    );
}

#[tokio::test]
async fn update_eval_extra_body_null_clears_the_name() {
    let harness = Harness::new();
    let mut calls = Calls::empty();
    calls.extra_body = map(&[("name", json("null"))]);
    let call = update_call(&calls, "eval_1", Some("original"), None);
    let mut state = harness.logging_state("aupdate_eval");
    let env = harness.env.pre_post();
    harness.handler.answer_update(Ok(eval("eval_1")));
    let update = EvalUpdate {
        call: &call,
        logging: &mut state,
        env: &env,
    };
    harness
        .service()
        .update_eval(update)
        .await
        .expect("the update succeeds");
    let updates = harness.handler.updates();
    assert_eq!(updates[0].body, object("{}"));
    assert_eq!(harness.recorded().updates[0].request.name, None);
}

#[tokio::test]
async fn update_eval_extra_body_null_drops_the_metadata() {
    let harness = Harness::new();
    let mut calls = Calls::empty();
    calls.extra_body = map(&[("metadata", json("null"))]);
    let metadata = map(&[("team", json("\"core\""))]);
    let call = update_call(&calls, "eval_1", None, Some(&metadata));
    let mut state = harness.logging_state("aupdate_eval");
    let env = harness.env.pre_post();
    harness.handler.answer_update(Ok(eval("eval_1")));
    let update = EvalUpdate {
        call: &call,
        logging: &mut state,
        env: &env,
    };
    harness
        .service()
        .update_eval(update)
        .await
        .expect("the update succeeds");
    let updates = harness.handler.updates();
    assert_eq!(updates[0].body, object("{}"));
    assert_eq!(harness.recorded().updates[0].request.metadata, None);
}

#[tokio::test]
async fn update_eval_extra_body_carries_unmodelled_fields() {
    let harness = Harness::new();
    let mut calls = Calls::empty();
    calls.extra_body = map(&[
        ("seed", json("42")),
        ("data_source_config", json("{\"type\":\"logs\"}")),
        ("dropped", json("null")),
    ]);
    let call = update_call(&calls, "eval_1", None, None);
    let mut state = harness.logging_state("aupdate_eval");
    let env = harness.env.pre_post();
    harness.handler.answer_update(Ok(eval("eval_1")));
    let update = EvalUpdate {
        call: &call,
        logging: &mut state,
        env: &env,
    };
    harness
        .service()
        .update_eval(update)
        .await
        .expect("the update succeeds");
    let updates = harness.handler.updates();
    assert_eq!(
        updates[0].body,
        object(r#"{"data_source_config":{"type":"logs"},"seed":42}"#)
    );
    assert!(!updates[0].body.contains_key("dropped"));
}

#[tokio::test]
async fn update_eval_extra_body_can_send_the_eval_id_in_the_body() {
    let harness = Harness::new();
    let mut calls = Calls::empty();
    calls.extra_body = map(&[("eval_id", json("\"other\""))]);
    let call = update_call(&calls, "eval/1", None, None);
    let mut state = harness.logging_state("aupdate_eval");
    let env = harness.env.pre_post();
    harness.handler.answer_update(Ok(eval("eval_1")));
    let update = EvalUpdate {
        call: &call,
        logging: &mut state,
        env: &env,
    };
    harness
        .service()
        .update_eval(update)
        .await
        .expect("the update succeeds");
    let updates = harness.handler.updates();
    assert_eq!(updates[0].body, object(r#"{"eval_id":"other"}"#));
    assert_eq!(updates[0].url, "https://api.openai.com/v1/evals/eval%2F1");
}

#[tokio::test]
async fn update_eval_accepts_extra_query_without_forwarding_it() {
    let harness = Harness::new();
    let mut calls = Calls::empty();
    calls.extra_query = map(&[("expand", json("\"results\""))]);
    let call = update_call(&calls, "eval_1", None, None);
    let mut state = harness.logging_state("aupdate_eval");
    let env = harness.env.pre_post();
    harness.handler.answer_update(Ok(eval("eval_1")));
    let update = EvalUpdate {
        call: &call,
        logging: &mut state,
        env: &env,
    };
    harness
        .service()
        .update_eval(update)
        .await
        .expect("the update succeeds");
    let updates = harness.handler.updates();
    assert_eq!(updates[0].url, "https://api.openai.com/v1/evals/eval_1");
    assert!(!updates[0].url.contains('?'));
    assert_eq!(updates[0].body, object("{}"));
}

#[tokio::test]
async fn update_eval_uses_the_kwargs_api_base() {
    let harness = Harness::new();
    let mut calls = Calls::empty();
    calls.kwargs = map(&[("api_base", json("\"http://upstream.test\""))]);
    let call = update_call(&calls, "eval_1", None, None);
    let mut state = harness.logging_state("aupdate_eval");
    let env = harness.env.pre_post();
    harness.handler.answer_update(Ok(eval("eval_1")));
    let update = EvalUpdate {
        call: &call,
        logging: &mut state,
        env: &env,
    };
    harness
        .service()
        .update_eval(update)
        .await
        .expect("the update succeeds");
    let updates = harness.handler.updates();
    assert_eq!(updates[0].url, "http://upstream.test/v1/evals/eval_1");
    assert_eq!(
        harness.recorded().updates[0].api_base,
        "http://upstream.test"
    );
}

#[tokio::test]
async fn update_eval_uses_an_explicit_empty_api_base_as_given() {
    let harness = Harness::new();
    let mut calls = Calls::empty();
    calls.kwargs = map(&[("api_base", json("\"\""))]);
    let call = update_call(&calls, "eval_1", None, None);
    let mut state = harness.logging_state("aupdate_eval");
    let env = harness.env.pre_post();
    harness.handler.answer_update(Ok(eval("eval_1")));
    let update = EvalUpdate {
        call: &call,
        logging: &mut state,
        env: &env,
    };
    harness
        .service()
        .update_eval(update)
        .await
        .expect("the update succeeds");
    let updates = harness.handler.updates();
    assert_eq!(updates[0].url, "/v1/evals/eval_1");
}
