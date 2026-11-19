use rstest::rstest;

use super::*;

const CREATE_DATA: &str = concat!(
    r#"{"completion_window":"24h","endpoint":"/v1/embeddings","input_file_id":"file-1","#,
    r#""metadata":{"team":"ml"},"output_expires_after":{"anchor":"created_at","seconds":3600}}"#
);

fn create_step() -> Step {
    creating(CREATE_DATA)
}

fn batch_reply() -> Option<Answered> {
    Some(Answered::Batch(Box::new(batch_object(&batch_body()))))
}

fn page_reply() -> Option<Answered> {
    Some(Answered::Page(Box::new(page_object(&page_body()))))
}

// Every create key reaches the SDK unchanged, the reply becomes the batch, and async accepts both client families
fn case_1_batch_create() {
    let azure = async_(create_step());
    let seen = observe(&azure);
    assert_eq!(seen.raised, None);
    assert_eq!(seen.reply, batch_reply());
    assert_eq!(
        seen.calls,
        vec![Recorded::Create(create_request(CREATE_DATA))]
    );
    assert_eq!(seen.seen[0].class, "AsyncAzureOpenAI");
    assert_eq!(
        seen.seen[0].base_url.as_deref(),
        Some("https://fixture.test/openai/")
    );

    let v1 = async_(versioned(create_step(), Some("v1")));
    let seen = observe(&v1);
    assert_eq!(seen.raised, None);
    assert_eq!(
        seen.calls,
        vec![Recorded::Create(create_request(CREATE_DATA))]
    );
    assert_eq!(seen.seen[0].class, "AsyncOpenAI");
    assert_eq!(
        seen.seen[0].base_url.as_deref(),
        Some("https://fixture.test/openai/v1/")
    );

    // a v1 AsyncOpenAI supplied by the caller is used as it is
    let supplied = supplying(
        async_(versioned(create_step(), Some("v1"))),
        OutcomeKind::AsyncOpenAi,
    );
    let seen = observe(&supplied);
    assert_eq!(seen.raised, None);
    assert_eq!(seen.calls.len(), 1);
    assert!(seen.seen[0].supplied);
}

// Retrieve and cancel forward their record, and list answers with the page the SDK gave
fn case_2_batch_retrieve_cancel_list() {
    let query = r#"{"batch_id":"batch_9","extra_headers":{"x-vendor":"a"},"extra_body":{"note":"n"},"timeout":12.5}"#;
    let retrieve = query_data(step(Action::Retrieve), query);
    let seen = observe(&retrieve);
    assert_eq!(seen.raised, None);
    assert_eq!(seen.reply, batch_reply());
    assert_eq!(seen.calls, vec![Recorded::Retrieve(query_request(query))]);

    let cancel = async_(query_data(step(Action::Cancel), query));
    let seen = observe(&cancel);
    assert_eq!(seen.raised, None);
    assert_eq!(seen.reply, batch_reply());
    assert_eq!(seen.calls, vec![Recorded::Cancel(query_request(query))]);
    assert_eq!(seen.seen[0].class, "AsyncAzureOpenAI");

    let list = listing(Some("batch_0"), Some(20));
    let seen = observe(&list);
    assert_eq!(seen.raised, None);
    assert_eq!(seen.reply, page_reply());
    assert_eq!(
        seen.calls,
        vec![Recorded::List {
            after: Some("batch_0".to_owned()),
            limit: Some(20),
        }]
    );
}

// The api_version argument picks the client family, and the resolved version rides on the Azure client
fn case_3_resource_api_version() {
    let azure = project(&step(Action::List)).expect("the Azure client projects");
    assert_eq!(azure.class, "AzureOpenAI");
    assert_eq!(azure.api_version.as_deref(), Some("2024-10-21"));
    assert_eq!(
        azure.base_url.as_deref(),
        Some("https://fixture.test/openai/")
    );
    assert_eq!(azure.api_key.as_deref(), Some("key-1"));
    assert_eq!(azure.timeout, None);
    assert_eq!(azure.max_retries, Some(2));

    for version in ["preview", "latest", "v1"] {
        let v1 =
            project(&versioned(step(Action::List), Some(version))).expect("the v1 client projects");
        assert_eq!(v1.class, "OpenAI");
        assert_eq!(v1.api_version, None);
        assert_eq!(
            v1.base_url.as_deref(),
            Some("https://fixture.test/openai/v1/")
        );
    }
    let stream = project(&async_(versioned(step(Action::List), Some("v1"))))
        .expect("the async v1 client projects");
    assert_eq!(stream.class, "AsyncOpenAI");

    let default =
        project(&versioned(step(Action::List), None)).expect("the default version projects");
    assert_eq!(default.class, "AzureOpenAI");
    assert_eq!(default.api_version.as_deref(), Some("2025-02-01-preview"));

    let empty =
        project(&versioned(step(Action::List), Some(""))).expect("the empty version projects");
    assert_eq!(empty.api_version.as_deref(), Some(""));

    let deployment = project(&endpoint_step("https://fixture.test/openai/deployments/d"))
        .expect("the deployment base projects");
    assert_eq!(
        deployment.base_url.as_deref(),
        Some("https://fixture.test/openai/deployments/d/")
    );
    assert_eq!(deployment.api_version.as_deref(), Some("2024-10-21"));
}

// List always passes the cursor pair on, as None when the caller gave none, and keeps the SDK page
fn case_4_list_after_limit() {
    let defaults = observe(&listing(None, None));
    assert_eq!(defaults.raised, None);
    assert_eq!(defaults.reply, page_reply());
    assert_eq!(
        defaults.calls,
        vec![Recorded::List {
            after: None,
            limit: None,
        }]
    );

    let explicit = observe(&listing(Some("batch_0"), Some(20)));
    assert_eq!(
        explicit.calls,
        vec![Recorded::List {
            after: Some("batch_0".to_owned()),
            limit: Some(20),
        }]
    );

    let limit_only = observe(&listing(None, Some(5)));
    assert_eq!(
        limit_only.calls,
        vec![Recorded::List {
            after: None,
            limit: Some(5),
        }]
    );

    let after_only = observe(&listing(Some("batch_7"), None));
    assert_eq!(
        after_only.calls,
        vec![Recorded::List {
            after: Some("batch_7".to_owned()),
            limit: None,
        }]
    );
}

// Every operation raises the same ValueError when the client builder returns no client
fn case_5_sdk_client_missing() {
    let cases = [
        missing(create_step()),
        missing(async_(query_data(
            step(Action::Retrieve),
            r#"{"batch_id":"batch_1"}"#,
        ))),
        missing(query_data(
            step(Action::Cancel),
            r#"{"batch_id":"batch_1"}"#,
        )),
        missing(async_(listing(None, None))),
    ];
    for case in &cases {
        let seen = observe(case);
        assert_eq!(seen.raised, Some(Raised::ClientMissing(MISSING.to_owned())));
        assert_eq!(seen.reply, None);
        assert!(seen.calls.is_empty());
        assert!(seen.seen.is_empty());
    }
}

// Create, retrieve and list name AsyncOpenAI; cancel names both Azure async classes
fn case_6_sdk_client_wrong_async() {
    let wrong = [
        async_(supplying(create_step(), OutcomeKind::OpenAi)),
        async_(supplying(
            query_data(step(Action::Retrieve), r#"{"batch_id":"batch_1"}"#),
            OutcomeKind::AzureOpenAi,
        )),
        async_(supplying(listing(None, None), OutcomeKind::OpenAi)),
    ];
    for case in &wrong {
        let seen = observe(case);
        assert_eq!(seen.raised, Some(Raised::NotAsync(NOT_ASYNC.to_owned())));
        assert!(seen.calls.is_empty());
    }

    let cancel = async_(supplying(
        query_data(step(Action::Cancel), r#"{"batch_id":"batch_1"}"#),
        OutcomeKind::OpenAi,
    ));
    let seen = observe(&cancel);
    assert_eq!(
        seen.raised,
        Some(Raised::CancelNotAsync(CANCEL_NOT_ASYNC.to_owned()))
    );
    assert!(seen.calls.is_empty());

    // the matching async families pass the same check
    let accepted = async_(supplying(create_step(), OutcomeKind::AsyncOpenAi));
    assert_eq!(observe(&accepted).raised, None);
    let accepted = async_(supplying(
        query_data(step(Action::Cancel), r#"{"batch_id":"batch_1"}"#),
        OutcomeKind::AsyncAzureOpenAi,
    ));
    assert_eq!(observe(&accepted).raised, None);
}

// Only cancel checks the sync class; the other sync calls hand the client on as it is
fn case_7_cancel_sync_client_type() {
    for kind in [OutcomeKind::AsyncOpenAi, OutcomeKind::AsyncAzureOpenAi] {
        let cancel = supplying(
            query_data(step(Action::Cancel), r#"{"batch_id":"batch_1"}"#),
            kind,
        );
        let seen = observe(&cancel);
        assert_eq!(seen.raised, Some(Raised::NotSync(NOT_SYNC.to_owned())));
        assert!(seen.calls.is_empty());
    }

    for kind in [OutcomeKind::OpenAi, OutcomeKind::AzureOpenAi] {
        let cancel = supplying(
            query_data(step(Action::Cancel), r#"{"batch_id":"batch_1"}"#),
            kind,
        );
        let seen = observe(&cancel);
        assert_eq!(seen.raised, None);
        assert_eq!(seen.calls.len(), 1);
    }

    // a sync create, retrieve or list handed an async client sends the request anyway
    let create = supplying(create_step(), OutcomeKind::AsyncOpenAi);
    let seen = observe(&create);
    assert_eq!(seen.raised, None);
    assert_eq!(
        seen.calls,
        vec![Recorded::Create(create_request(CREATE_DATA))]
    );

    let retrieve = supplying(
        query_data(step(Action::Retrieve), r#"{"batch_id":"batch_1"}"#),
        OutcomeKind::AsyncOpenAi,
    );
    let seen = observe(&retrieve);
    assert_eq!(seen.raised, None);
    assert_eq!(seen.calls.len(), 1);

    let list = supplying(listing(None, None), OutcomeKind::AsyncOpenAi);
    let seen = observe(&list);
    assert_eq!(seen.raised, None);
    assert_eq!(seen.reply, page_reply());
}

// Every SDK failure reaches the caller unchanged, and only litellm_params reaches the client
fn case_8_sdk_error_passthrough() {
    let failure = status_failure();
    let failing_step = failing(async_(create_step()), failure.clone());
    let seen = observe(&failing_step);
    assert_eq!(
        seen.raised,
        Some(Raised::Status {
            status: failure.status,
            error_type: failure.error_type.clone(),
            param: failure.param.clone(),
            code: failure.code.clone(),
            message: failure.message.clone(),
        })
    );
    assert_eq!(seen.reply, None);
    assert_eq!(
        seen.calls,
        vec![Recorded::Create(create_request(CREATE_DATA))]
    );

    let broken = timed_out(create_step(), 0.5);
    let seen = observe(&broken);
    assert_eq!(
        seen.raised,
        Some(Raised::Sdk(
            "the timeout must be a finite, non-negative number of seconds, not 0.5".to_owned()
        ))
    );

    // timeout, max_retries and the ad token reach the client from litellm_params alone
    let tuned = with_params(
        create_step(),
        r#"{"timeout":30.0,"max_retries":5,"azure_ad_token":"ad-token"}"#,
    );
    let seen = observe(&tuned);
    assert_eq!(seen.raised, None);
    assert_eq!(seen.seen[0].timeout, Some(30.0));
    assert_eq!(seen.seen[0].max_retries, Some(5));
    assert_eq!(seen.seen[0].ad_token.as_deref(), Some("ad-token"));

    let untouched = observe(&create_step());
    assert_eq!(untouched.seen[0].timeout, None);
    assert_eq!(untouched.seen[0].max_retries, Some(2));
    assert_eq!(untouched.seen[0].ad_token, None);
}

#[rstest]
#[case::batch_create(case_1_batch_create)]
#[case::batch_retrieve_cancel_list(case_2_batch_retrieve_cancel_list)]
#[case::resource_api_version(case_3_resource_api_version)]
#[case::list_after_limit(case_4_list_after_limit)]
#[case::sdk_client_missing(case_5_sdk_client_missing)]
#[case::sdk_client_wrong_async(case_6_sdk_client_wrong_async)]
#[case::cancel_sync_client_type(case_7_cancel_sync_client_type)]
#[case::sdk_error_passthrough(case_8_sdk_error_passthrough)]
fn batches_cases(#[case] case: fn()) {
    case();
}
