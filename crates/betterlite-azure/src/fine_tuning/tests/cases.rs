use rstest::rstest;

use super::*;

const NO_TRAINING_TYPE: &str = r#"{"model":"m","training_file":"f"}"#;
const DEFAULTED: &str = r#"{"model":"m","training_file":"f","trainingType":1}"#;
const DEFAULTED_SEEDED: &str = r#"{"model":"m","training_file":"f","seed":4,"trainingType":1}"#;
const DEFAULTED_WITH_EXTRA: &str =
    r#"{"model":"m","training_file":"f","prompt_loss_weight":0.1,"trainingType":1}"#;
const EXPLICIT_TRAINING_TYPE: &str =
    r#"{"model":"m","training_file":"f","extra_body":{"trainingType":2}}"#;
const CREATE_ERROR: &str = r#"{"error":{"message":"bad job","type":"invalid_request_error","param":"training_file","code":"invalid"}}"#;
const CANCEL_ERROR: &str = r#"{"error":{"message":"no such job","type":"invalid_request_error","param":null,"code":"not_found"}}"#;

#[rstest]
#[case::training_type_default(training_type_default)]
#[case::explicit_training_type(explicit_training_type)]
#[case::create_cancel_retrieve(create_cancel_retrieve)]
#[case::azure_status_normalization(azure_status_normalization)]
#[case::organization_result_files_null(organization_result_files_null)]
#[case::azure_finetune_client_rebuild(azure_finetune_client_rebuild)]
#[case::sdk_client_missing(sdk_client_missing)]
#[case::sdk_client_wrong_async(sdk_client_wrong_async)]
#[case::sdk_error_passthrough(sdk_error_passthrough)]
fn fine_tuning_cases(#[case] case: fn()) {
    case();
}

fn training_type_default() {
    let body = job_json("queued", ORG, "[]");
    let (observation, messages) = capture(|| run(&create(NO_TRAINING_TYPE, &body)));
    assert_eq!(
        observation,
        job_observation(&body, vec![canonical_create(DEFAULTED)])
    );
    assert!(messages.iter().any(|message| message == DEBUG_DEFAULT));
    let (_, explicit) = capture(|| run(&create(EXPLICIT_TRAINING_TYPE, &body)));
    assert!(!explicit.iter().any(|message| message == DEBUG_DEFAULT));
    training_type_defaults_from_every_empty_extra_body();
}

fn training_type_defaults_from_every_empty_extra_body() {
    let body = job_json("queued", ORG, "[]");
    let steps = [
        r#"{"model":"m","training_file":"f","extra_body":null}"#,
        r#"{"model":"m","training_file":"f","extra_body":{}}"#,
        r#"{"model":"m","training_file":"f","extra_body":0}"#,
        r#"{"model":"m","training_file":"f","extra_body":false}"#,
        r#"{"model":"m","training_file":"f","extra_body":""}"#,
        r#"{"model":"m","training_file":"f","extra_body":[]}"#,
        r#"{"model":"m","training_file":"f","extra_body":"text"}"#,
        r#"{"model":"m","training_file":"f","extra_body":{"prompt_loss_weight":0.1}}"#,
        r#"{"model":"m","training_file":"f","extra_body":{"trainingType":null,"prompt_loss_weight":0.1}}"#,
    ];
    for step in &steps {
        let observation = run(&create(step, &body));
        assert_eq!(observation.outcome, Outcome::Job(canonical_job(&body)));
    }
    assert_eq!(
        run(&create(steps[7], &body)).calls,
        vec![canonical_create(DEFAULTED_WITH_EXTRA)]
    );
    assert_eq!(
        run(&create(steps[8], &body)).calls,
        vec![canonical_create(DEFAULTED_WITH_EXTRA)]
    );
}

fn explicit_training_type() {
    let body = job_json("queued", ORG, "[]");
    let steps = [
        (
            r#"{"model":"m","training_file":"f","extra_body":{"trainingType":0}}"#,
            r#"{"model":"m","training_file":"f","trainingType":0}"#,
        ),
        (
            r#"{"model":"m","training_file":"f","extra_body":{"trainingType":false}}"#,
            r#"{"model":"m","training_file":"f","trainingType":false}"#,
        ),
        (
            r#"{"model":"m","training_file":"f","extra_body":{"trainingType":""}}"#,
            r#"{"model":"m","training_file":"f","trainingType":""}"#,
        ),
        (
            r#"{"model":"m","training_file":"f","extra_body":{"trainingType":2,"note":"kept"}}"#,
            r#"{"model":"m","training_file":"f","note":"kept","trainingType":2}"#,
        ),
    ];
    for (data, expected) in steps {
        let observation = run(&create(data, &body));
        assert_eq!(observation.outcome, Outcome::Job(canonical_job(&body)));
        assert_eq!(observation.calls, vec![canonical_create(expected)]);
    }
}

fn create_cancel_retrieve() {
    let (steps, expected) = create_cancel_retrieve_plan();
    let actual: Vec<Observation> = steps.iter().map(run).collect();
    assert_eq!(actual, expected);
    debug_logs_only_on_sync_paths();
}

fn create_cancel_retrieve_plan() -> (Vec<Step>, Vec<Observation>) {
    let queued = job_json("queued", ORG, "[]");
    let cancelled = job_json("cancelled", ORG, "[]");
    let null_error = job_json("queued", ORG, "[]").replace(
        r#""error":null"#,
        r#""error":{"code":null,"message":null,"param":null}"#,
    );
    let steps = vec![
        create(NO_TRAINING_TYPE, &null_error),
        async_(create(
            r#"{"model":"m","training_file":"f","seed":4}"#,
            &queued,
        )),
        retrieve("ft_retrieve", &queued),
        async_(cancel("ft_cancel", &cancelled)),
    ];
    let expected = vec![
        job_observation(&queued, vec![canonical_create(DEFAULTED)]),
        job_observation(&queued, vec![canonical_create(DEFAULTED_SEEDED)]),
        job_observation(
            &queued,
            vec![Call::Retrieve {
                job_id: "ft_retrieve".to_owned(),
            }],
        ),
        job_observation(
            &cancelled,
            vec![Call::Cancel {
                job_id: "ft_cancel".to_owned(),
            }],
        ),
    ];
    (steps, expected)
}

fn debug_logs_only_on_sync_paths() {
    let queued = job_json("queued", ORG, "[]");
    let (_, sync_create) = capture(|| run(&create(NO_TRAINING_TYPE, &queued)));
    let (_, async_create) = capture(|| {
        run(&async_(create(
            r#"{"model":"m","training_file":"f","seed":4}"#,
            &queued,
        )))
    });
    let (_, sync_list) = capture(|| run(&after(list(&queued), "ft_1", 7)));
    let (_, async_list) = capture(|| run(&async_(after(list(&queued), "ft_2", 3))));
    assert!(
        sync_create
            .iter()
            .any(|message| message.starts_with(DEBUG_CREATE))
    );
    assert!(
        !async_create
            .iter()
            .any(|message| message.starts_with(DEBUG_CREATE))
    );
    assert!(
        sync_list
            .iter()
            .any(|message| message.starts_with(DEBUG_LIST))
    );
    assert!(
        !async_list
            .iter()
            .any(|message| message.starts_with(DEBUG_LIST))
    );
}

fn azure_status_normalization() {
    azure_statuses_map_to_openai();
    list_pages_keep_azure_status_values();
    unknown_status_is_a_validation_error();
}

fn azure_statuses_map_to_openai() {
    let mapped = [
        ("pending", "queued"),
        ("notRunning", "queued"),
        ("running", "running"),
        ("succeeded", "succeeded"),
        ("failed", "failed"),
        ("canceled", "cancelled"),
        ("canceling", "cancelled"),
    ];
    for (azure, openai) in mapped {
        let reply = job_json(azure, ORG, "[]");
        let observation = run(&retrieve("ft_status", &reply));
        assert_eq!(
            observation.outcome,
            Outcome::Job(canonical_job(&job_json(openai, ORG, "[]")))
        );
    }
}

fn list_pages_keep_azure_status_values() {
    let pending_page = page(&job_json("pending", r#"null"#, "null"), true);
    let observed = run(&list(&pending_page));
    assert_eq!(observed.outcome, Outcome::Page(pending_page.clone()));
}

fn unknown_status_is_a_validation_error() {
    let unknown = job_json("paused", ORG, "[]");
    let observation = run(&retrieve("ft_paused", &unknown));
    assert_eq!(
        observation,
        Observation {
            outcome: Outcome::Raised(validation_raised()),
            calls: vec![Call::Retrieve {
                job_id: "ft_paused".to_owned(),
            }],
            supplied_calls: Vec::new(),
        }
    );
}

fn organization_result_files_null() {
    let nulled = job_json("queued", "null", "null");
    let missing = r#"{"id":"ft_1","created_at":1,"error":null,"fine_tuned_model":null,"finished_at":null,"estimated_finish":null,"integrations":null,"metadata":null,"hyperparameters":{"batch_size":null,"learning_rate_multiplier":null,"n_epochs":3},"method":null,"model":"gpt-4o-mini","object":"fine_tuning.job","seed":5,"status":"queued","trained_tokens":null,"training_file":"file_a","validation_file":null}"#;
    let filled = job_json("queued", ORG, r#"["file_r"]"#);
    let steps = [
        create(NO_TRAINING_TYPE, &nulled),
        retrieve("ft_missing", missing),
        cancel("ft_filled", &filled),
    ];
    let expected_bodies = [
        job_json("queued", r#""""#, "[]"),
        job_json("queued", r#""""#, "[]"),
        filled.clone(),
    ];
    for (step, expected) in steps.iter().zip(expected_bodies.iter()) {
        let observation = run(step);
        assert_eq!(observation.outcome, Outcome::Job(canonical_job(expected)));
    }
    let null_page = page(&nulled, false);
    let observed = run(&list(&null_page));
    assert_eq!(observed.outcome, Outcome::Page(null_page));
}

fn azure_finetune_client_rebuild() {
    for (step, class) in projection_cases() {
        let observation = run(&step);
        assert_eq!(observation.outcome, projected(class));
        assert!(observation.calls.is_empty());
        assert!(observation.supplied_calls.is_empty());
    }
    let queued = job_json("queued", ORG, "[]");
    let rebuilt = run(&supplied(
        create(NO_TRAINING_TYPE, &queued),
        Supplied::AzureOpenAi,
    ));
    assert_eq!(
        rebuilt,
        job_observation(&queued, vec![canonical_create(DEFAULTED)])
    );
}

fn projection_cases() -> [(Step, &'static str); 4] {
    [
        (project(Supplied::OpenAi, false), "AzureOpenAI"),
        (project(Supplied::AsyncOpenAi, false), "AzureOpenAI"),
        (project(Supplied::AzureOpenAi, true), "AsyncAzureOpenAI"),
        (project(Supplied::AsyncAzureOpenAi, false), "AzureOpenAI"),
    ]
}

fn projected(class: &'static str) -> Outcome {
    Outcome::Projection(Projection {
        class,
        api_key: Some("key-1".to_owned()),
        max_retries: Some(2),
        timeout: None,
        organization: None,
    })
}

fn sdk_client_missing() {
    let queued = job_json("queued", ORG, "[]");
    let steps = [
        built_by(create(NO_TRAINING_TYPE, &queued), BuilderMode::None),
        built_by(retrieve("ft_1", &queued), BuilderMode::None),
        built_by(cancel("ft_1", &queued), BuilderMode::None),
        built_by(list(&page(&queued, false)), BuilderMode::None),
    ];
    for step in &steps {
        assert_eq!(run(step), raised_value(MISSING));
    }
}

fn sdk_client_wrong_async() {
    for step in wrong_async_steps() {
        assert_eq!(run(&step), raised_value(NOT_ASYNC));
    }
    let queued = job_json("queued", ORG, "[]");
    let sync = built_by(create(NO_TRAINING_TYPE, &queued), BuilderMode::WrongAsync);
    assert_eq!(
        run(&sync),
        job_observation(&queued, vec![canonical_create(DEFAULTED)])
    );
}

fn wrong_async_steps() -> [Step; 4] {
    let queued = job_json("queued", ORG, "[]");
    [
        async_(built_by(
            create(NO_TRAINING_TYPE, &queued),
            BuilderMode::WrongAsync,
        )),
        async_(built_by(retrieve("ft_1", &queued), BuilderMode::WrongAsync)),
        async_(built_by(cancel("ft_1", &queued), BuilderMode::WrongAsync)),
        async_(built_by(
            list(&page(&queued, false)),
            BuilderMode::WrongAsync,
        )),
    ]
}

fn sdk_error_passthrough() {
    sdk_errors_keep_their_fields();
    list_replies_are_passed_through();
    debug_logs_only_on_sync_paths();
    unknown_status_is_a_validation_error();
}

fn sdk_errors_keep_their_fields() {
    let queued = job_json("queued", ORG, "[]");
    let created = Step {
        reply: Some(failure(400, CREATE_ERROR)),
        ..create(NO_TRAINING_TYPE, &queued)
    };
    assert_sdk_error(
        &created,
        status_raised(400, "bad job", Some("training_file"), "invalid"),
        vec![canonical_create(DEFAULTED)],
    );
    let cancelled = Step {
        reply: Some(failure(404, CANCEL_ERROR)),
        ..cancel("ft_cancel", &queued)
    };
    assert_sdk_error(
        &cancelled,
        status_raised(404, "no such job", None, "not_found"),
        vec![Call::Cancel {
            job_id: "ft_cancel".to_owned(),
        }],
    );
}

fn assert_sdk_error(step: &Step, error: Raised, calls: Vec<Call>) {
    assert_eq!(
        run(step),
        Observation {
            outcome: Outcome::Raised(error),
            calls,
            supplied_calls: Vec::new(),
        }
    );
}

fn list_replies_are_passed_through() {
    let queued = job_json("queued", ORG, "[]");
    let sync_page = page(&queued, true);
    assert_eq!(
        run(&after(list(&sync_page), "ft_1", 7)),
        Observation {
            outcome: Outcome::Page(sync_page),
            calls: vec![Call::List {
                after: Some("ft_1".to_owned()),
                limit: Some(7),
            }],
            supplied_calls: Vec::new(),
        }
    );
    let async_page = page(&queued, true);
    assert_eq!(
        run(&async_(after(list(&async_page), "ft_2", 3))),
        Observation {
            outcome: Outcome::Page(async_page),
            calls: vec![Call::List {
                after: Some("ft_2".to_owned()),
                limit: Some(3),
            }],
            supplied_calls: Vec::new(),
        }
    );
}

fn raised_value(message: &str) -> Observation {
    Observation {
        outcome: Outcome::Raised(Raised {
            class: "ValueError",
            message: Some(message.to_owned()),
            status: None,
            error_type: None,
            param: None,
            code: None,
        }),
        calls: Vec::new(),
        supplied_calls: Vec::new(),
    }
}

fn validation_raised() -> Raised {
    Raised {
        class: "ValidationError",
        message: None,
        status: None,
        error_type: None,
        param: None,
        code: None,
    }
}

fn status_raised(status: u16, message: &str, param: Option<&str>, code: &str) -> Raised {
    Raised {
        class: "APIStatusError",
        message: Some(message.to_owned()),
        status: Some(status),
        error_type: Some("invalid_request_error".to_owned()),
        param: param.map(str::to_owned),
        code: Some(code.to_owned()),
    }
}
