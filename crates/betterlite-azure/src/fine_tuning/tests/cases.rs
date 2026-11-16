use rstest::rstest;

use super::*;

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
    let steps = [
        r#"{"model":"m","training_file":"f"}"#,
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
    let (first, messages) = capture(|| run(&create(steps[0], &body)));
    assert_eq!(
        first,
        job_observation(
            &body,
            vec![canonical_create(
                r#"{"model":"m","training_file":"f","trainingType":1}"#
            )]
        )
    );
    assert!(messages.iter().any(|message| message == DEBUG_DEFAULT));
    for step in &steps[1..] {
        let observation = run(&create(step, &body));
        assert_eq!(observation.outcome, Outcome::Job(canonical_job(&body)));
    }
    assert_eq!(
        run(&create(steps[8], &body)).calls,
        vec![canonical_create(
            r#"{"model":"m","training_file":"f","prompt_loss_weight":0.1,"trainingType":1}"#
        )]
    );
    assert_eq!(
        run(&create(steps[9], &body)).calls,
        vec![canonical_create(
            r#"{"model":"m","training_file":"f","prompt_loss_weight":0.1,"trainingType":1}"#
        )]
    );
    let (_, explicit) = capture(|| {
        run(&create(
            r#"{"model":"m","training_file":"f","extra_body":{"trainingType":2}}"#,
            &body,
        ))
    });
    assert!(!explicit.iter().any(|message| message == DEBUG_DEFAULT));
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
    let queued = job_json("queued", ORG, "[]");
    let cancelled = job_json("cancelled", ORG, "[]");
    let null_error = job_json("queued", ORG, "[]").replace(
        r#""error":null"#,
        r#""error":{"code":null,"message":null,"param":null}"#,
    );
    let steps = vec![
        create(r#"{"model":"m","training_file":"f"}"#, &null_error),
        async_(create(
            r#"{"model":"m","training_file":"f","seed":4}"#,
            &queued,
        )),
        retrieve("ft_retrieve", &queued),
        async_(cancel("ft_cancel", &cancelled)),
    ];
    let expected = [
        job_observation(
            &queued,
            vec![canonical_create(
                r#"{"model":"m","training_file":"f","trainingType":1}"#,
            )],
        ),
        job_observation(
            &queued,
            vec![canonical_create(
                r#"{"model":"m","training_file":"f","seed":4,"trainingType":1}"#,
            )],
        ),
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
    let actual: Vec<Observation> = steps.iter().map(run).collect();
    assert_eq!(actual, expected.to_vec());
    let (_, sync_messages) = capture(|| run(&steps[0]));
    assert!(
        sync_messages
            .iter()
            .any(|message| message.starts_with(DEBUG_CREATE))
    );
    let (_, async_messages) = capture(|| run(&steps[1]));
    assert!(
        !async_messages
            .iter()
            .any(|message| message.starts_with(DEBUG_CREATE))
    );
}

fn azure_status_normalization() {
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
    let pending_page = page(&job_json("pending", r#"null"#, "null"), true);
    let observed = run(&list(&pending_page));
    assert_eq!(observed.outcome, Outcome::Page(pending_page.clone()));
    let unknown = job_json("paused", ORG, "[]");
    let observation = run(&retrieve("ft_paused", &unknown));
    assert_eq!(
        observation.outcome,
        Outcome::Raised(Raised {
            class: "ValidationError",
            message: None,
            status: None,
            error_type: None,
            param: None,
            code: None,
        })
    );
    assert_eq!(
        observation.calls,
        vec![Call::Retrieve {
            job_id: "ft_paused".to_owned(),
        }]
    );
}

fn organization_result_files_null() {
    let nulled = job_json("queued", "null", "null");
    let missing = r#"{"id":"ft_1","created_at":1,"error":null,"fine_tuned_model":null,"finished_at":null,"estimated_finish":null,"integrations":null,"metadata":null,"hyperparameters":{"batch_size":null,"learning_rate_multiplier":null,"n_epochs":3},"method":null,"model":"gpt-4o-mini","object":"fine_tuning.job","seed":5,"status":"queued","trained_tokens":null,"training_file":"file_a","validation_file":null}"#;
    let filled = job_json("queued", ORG, r#"["file_r"]"#);
    let steps = [
        create(r#"{"model":"m","training_file":"f"}"#, &nulled),
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
    let queued = job_json("queued", ORG, "[]");
    let projections = [
        project(Supplied::OpenAi, false),
        project(Supplied::AsyncOpenAi, false),
        project(Supplied::AzureOpenAi, true),
        project(Supplied::AsyncAzureOpenAi, false),
    ];
    let expected_classes = [
        "AzureOpenAI",
        "AzureOpenAI",
        "AsyncAzureOpenAI",
        "AzureOpenAI",
    ];
    for (step, class) in projections.iter().zip(expected_classes.iter()) {
        let observation = run(step);
        assert_eq!(
            observation.outcome,
            Outcome::Projection(Projection {
                class,
                api_key: Some("key-1".to_owned()),
                max_retries: Some(2),
                timeout: None,
                organization: None,
            })
        );
        assert!(observation.calls.is_empty());
        assert!(observation.supplied_calls.is_empty());
    }
    let observation = run(&supplied(
        create(r#"{"model":"m","training_file":"f"}"#, &queued),
        Supplied::AzureOpenAi,
    ));
    assert_eq!(
        observation,
        job_observation(
            &queued,
            vec![canonical_create(
                r#"{"model":"m","training_file":"f","trainingType":1}"#
            )]
        )
    );
}

fn sdk_client_missing() {
    let steps = [
        built_by(
            create(
                r#"{"model":"m","training_file":"f"}"#,
                &job_json("queued", ORG, "[]"),
            ),
            BuilderMode::None,
        ),
        built_by(
            retrieve("ft_1", &job_json("queued", ORG, "[]")),
            BuilderMode::None,
        ),
        built_by(
            cancel("ft_1", &job_json("queued", ORG, "[]")),
            BuilderMode::None,
        ),
        built_by(
            list(&page(&job_json("queued", ORG, "[]"), false)),
            BuilderMode::None,
        ),
    ];
    for step in &steps {
        assert_eq!(
            run(step),
            Observation {
                outcome: Outcome::Raised(Raised {
                    class: "ValueError",
                    message: Some(MISSING.to_owned()),
                    status: None,
                    error_type: None,
                    param: None,
                    code: None,
                }),
                calls: Vec::new(),
                supplied_calls: Vec::new(),
            }
        );
    }
}

fn sdk_client_wrong_async() {
    let queued = job_json("queued", ORG, "[]");
    let steps = [
        async_(built_by(
            create(r#"{"model":"m","training_file":"f"}"#, &queued),
            BuilderMode::WrongAsync,
        )),
        async_(built_by(retrieve("ft_1", &queued), BuilderMode::WrongAsync)),
        async_(built_by(cancel("ft_1", &queued), BuilderMode::WrongAsync)),
        async_(built_by(
            list(&page(&queued, false)),
            BuilderMode::WrongAsync,
        )),
    ];
    for step in &steps {
        assert_eq!(
            run(step),
            Observation {
                outcome: Outcome::Raised(Raised {
                    class: "ValueError",
                    message: Some(NOT_ASYNC.to_owned()),
                    status: None,
                    error_type: None,
                    param: None,
                    code: None,
                }),
                calls: Vec::new(),
                supplied_calls: Vec::new(),
            }
        );
    }
    let sync = built_by(
        create(r#"{"model":"m","training_file":"f"}"#, &queued),
        BuilderMode::WrongAsync,
    );
    assert_eq!(
        run(&sync),
        job_observation(
            &queued,
            vec![canonical_create(
                r#"{"model":"m","training_file":"f","trainingType":1}"#
            )]
        )
    );
}

fn sdk_error_passthrough() {
    let queued = job_json("queued", ORG, "[]");
    let invalid = failure(
        400,
        r#"{"error":{"message":"bad job","type":"invalid_request_error","param":"training_file","code":"invalid"}}"#,
    );
    let missing = failure(
        404,
        r#"{"error":{"message":"no such job","type":"invalid_request_error","param":null,"code":"not_found"}}"#,
    );
    let observation = run(&Step {
        reply: Some(invalid),
        ..create(r#"{"model":"m","training_file":"f"}"#, &queued)
    });
    assert_eq!(
        observation,
        Observation {
            outcome: Outcome::Raised(Raised {
                class: "APIStatusError",
                message: Some("bad job".to_owned()),
                status: Some(400),
                error_type: Some("invalid_request_error".to_owned()),
                param: Some("training_file".to_owned()),
                code: Some("invalid".to_owned()),
            }),
            calls: vec![canonical_create(
                r#"{"model":"m","training_file":"f","trainingType":1}"#
            )],
            supplied_calls: Vec::new(),
        }
    );
    let observation = run(&Step {
        reply: Some(missing),
        ..cancel("ft_cancel", &queued)
    });
    assert_eq!(
        observation,
        Observation {
            outcome: Outcome::Raised(Raised {
                class: "APIStatusError",
                message: Some("no such job".to_owned()),
                status: Some(404),
                error_type: Some("invalid_request_error".to_owned()),
                param: None,
                code: Some("not_found".to_owned()),
            }),
            calls: vec![Call::Cancel {
                job_id: "ft_cancel".to_owned(),
            }],
            supplied_calls: Vec::new(),
        }
    );
    let paused = job_json("paused", ORG, "[]");
    assert_eq!(
        run(&retrieve("ft_paused", &paused)),
        Observation {
            outcome: Outcome::Raised(Raised {
                class: "ValidationError",
                message: None,
                status: None,
                error_type: None,
                param: None,
                code: None,
            }),
            calls: vec![Call::Retrieve {
                job_id: "ft_paused".to_owned(),
            }],
            supplied_calls: Vec::new(),
        }
    );
    let observation = run(&after(list(&page(&queued, true)), "ft_1", 7));
    assert_eq!(
        observation,
        Observation {
            outcome: Outcome::Page(page(&queued, true)),
            calls: vec![Call::List {
                after: Some("ft_1".to_owned()),
                limit: Some(7),
            }],
            supplied_calls: Vec::new(),
        }
    );
    let (_, sync_messages) = capture(|| run(&after(list(&page(&queued, true)), "ft_1", 7)));
    assert!(
        sync_messages
            .iter()
            .any(|message| message.starts_with(DEBUG_LIST))
    );
    let (_, async_messages) =
        capture(|| run(&async_(after(list(&page(&queued, true)), "ft_2", 3))));
    assert!(
        !async_messages
            .iter()
            .any(|message| message.starts_with(DEBUG_LIST))
    );
}
