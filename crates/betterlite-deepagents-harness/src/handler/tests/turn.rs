#[tokio::test]
async fn turn_yields_only_model_node_messages() {
    let (runtime, plan) = FakeRuntime::new();
    plan.passes.lock().unwrap().push_back(vec![
        Ok(text_part("hello", "model")),
        Ok(text_part("echo", "tools")),
    ]);
    let mut handler = handler(runtime);
    let mut ctx = context("/work");
    handler.start(&mut ctx).await.unwrap();
    let events = run_turn(&mut handler, &mut ctx, "hi", |_| Decision::Allow).await;
    assert_eq!(deltas(&events), ["hello"]);
}

fn answer_and_read_pass() -> Vec<Result<StreamPart, HarnessError>> {
    let mut called = ai("thinking");
    called.tool_calls = vec![
        ToolCall {
            id: "1".to_owned(),
            name: "Answer".to_owned(),
            args: JsonObject::new(),
        },
        ToolCall {
            id: "2".to_owned(),
            name: "read_file".to_owned(),
            args: json_object(r#"{"path":"a.txt"}"#),
        },
    ];
    let mut result = ai("{}");
    result.kind = MessageKind::Tool;
    result.name = Some("Answer".to_owned());
    result.tool_call_id = "1".to_owned();
    vec![Ok(update_part(update_with(vec![
        ("model", UpdateEntry::Messages(vec![called])),
        ("tools", UpdateEntry::Messages(vec![result])),
    ])))]
}

#[tokio::test]
async fn turn_yields_update_events_without_the_skipped_tools() {
    let (runtime, plan) = FakeRuntime::new();
    plan.passes
        .lock()
        .unwrap()
        .push_back(answer_and_read_pass());
    let mut handler = handler(runtime);
    let mut ctx = context("/work");
    ctx.settings.output_schema = Some(json_object(r#"{"title":"Answer","type":"object"}"#));
    handler.start(&mut ctx).await.unwrap();
    let events = run_turn(&mut handler, &mut ctx, "hi", |_| Decision::Allow).await;
    let calls: Vec<(&str, &str)> = events
        .iter()
        .filter_map(|event| {
            let Event::ToolCall {
                name, native_name, ..
            } = event
            else {
                return None;
            };
            Some((name.as_str(), native_name.as_str()))
        })
        .collect();
    assert_eq!(calls, [("read", "read_file")]);
}

#[tokio::test]
async fn turn_passes_thread_id_recursion_limit_and_usage_callback() {
    let (runtime, plan) = FakeRuntime::new();
    plan.passes
        .lock()
        .unwrap()
        .push_back(vec![Ok(text_part("done", "model"))]);
    let mut handler = handler(runtime);
    let mut ctx = context("/work");
    ctx.settings.max_turns = Some(2);
    handler.start(&mut ctx).await.unwrap();
    run_turn(&mut handler, &mut ctx, "hi", |_| Decision::Allow).await;
    let config = plan.run_configs.lock().unwrap().first().unwrap().clone();
    assert_eq!(config.thread_id, "session-1");
    let expected_limit =
        BASE_RECURSION_LIMIT.saturating_add(2_u32.saturating_mul(DEEPAGENTS_STEPS_PER_TURN));
    assert_eq!(config.recursion_limit, expected_limit);
    assert!(config.usage_callback.is_some());
    let cost_models = plan.cost_models.lock().unwrap().clone();
    assert_eq!(cost_models.len(), 1);
    assert_eq!(
        cost_models.first().unwrap().as_deref(),
        Some("openai/gpt-4o")
    );
    let state_config = plan.state_configs.lock().unwrap().first().unwrap().clone();
    assert_eq!(state_config.thread_id, "session-1");
    assert!(state_config.usage_callback.is_some());
}

#[tokio::test]
async fn turn_uses_the_resumed_thread_id() {
    let (runtime, plan) = FakeRuntime::new();
    plan.passes
        .lock()
        .unwrap()
        .push_back(vec![Ok(text_part("done", "model"))]);
    let mut handler = handler(runtime);
    let mut ctx = context("/work");
    handler.start(&mut ctx).await.unwrap();
    handler.resume(&mut ctx, "native-7").await.unwrap();
    run_turn(&mut handler, &mut ctx, "hi", |_| Decision::Allow).await;
    assert_eq!(
        plan.run_configs.lock().unwrap().first().unwrap().thread_id,
        "native-7"
    );
}

#[tokio::test]
async fn turn_sets_final_text_and_structured_output() {
    let (runtime, plan) = FakeRuntime::new();
    plan.passes
        .lock()
        .unwrap()
        .push_back(vec![Ok(text_part("draft", "model"))]);
    plan.state.lock().unwrap().messages = vec![human("hi"), ai("the answer")];
    plan.state.lock().unwrap().structured_response = Some(json_object(r#"{"result":7}"#));
    let mut handler = handler(runtime);
    let mut ctx = context("/work");
    ctx.settings.output_schema = Some(json_object(r#"{"title":"Answer","type":"object"}"#));
    handler.start(&mut ctx).await.unwrap();
    let events = run_turn(&mut handler, &mut ctx, "hi", |_| Decision::Allow).await;
    assert_eq!(deltas(&events), ["draft"]);
    assert_eq!(ctx.outcome.final_text, "the answer");
    assert_eq!(ctx.outcome.output_json, Some(r#"{"result":7}"#.to_owned()));
}

#[tokio::test]
async fn turn_leaves_output_json_empty_without_an_output_schema() {
    let (runtime, plan) = FakeRuntime::new();
    plan.passes
        .lock()
        .unwrap()
        .push_back(vec![Ok(text_part("draft", "model"))]);
    plan.state.lock().unwrap().messages = vec![human("hi"), ai("the answer")];
    plan.state.lock().unwrap().structured_response = Some(json_object(r#"{"result":7}"#));
    let mut handler = handler(runtime);
    let mut ctx = context("/work");
    handler.start(&mut ctx).await.unwrap();
    run_turn(&mut handler, &mut ctx, "hi", |_| Decision::Allow).await;
    assert_eq!(ctx.outcome.final_text, "the answer");
    assert!(ctx.outcome.output_json.is_none());
}

#[tokio::test]
async fn turn_ends_with_the_error_a_graph_pass_fails_with() {
    let (runtime, plan) = FakeRuntime::new();
    plan.passes.lock().unwrap().push_back(vec![
        Ok(text_part("before", "model")),
        Err(HarnessError::SessionClosed),
    ]);
    let mut handler = handler(runtime);
    let mut ctx = context("/work");
    handler.start(&mut ctx).await.unwrap();
    let mut stream = Box::pin(handler.turn(&mut ctx, "hi"));
    let first = stream.next().await.unwrap().unwrap();
    assert!(matches!(first, Event::Text { delta } if delta == "before"));
    let second = stream.next().await.unwrap();
    assert!(matches!(second, Err(HarnessError::SessionClosed)));
    assert!(stream.next().await.is_none());
    assert!(plan.state_configs.lock().unwrap().is_empty());
}

async fn run_write_then_bash_turn() -> (Arc<Plan>, Vec<Event>) {
    let (runtime, plan) = FakeRuntime::new();
    let mut called = ai("checking files");
    called.tool_calls = vec![ToolCall {
        id: "1".to_owned(),
        name: "write_file".to_owned(),
        args: json_object(r#"{"path":"a.txt"}"#),
    }];
    let mut interrupted = interrupt_update(
        "int-1",
        &[("write_file", r#"{"path":"a.txt"}"#), ("execute", "{}")],
    );
    interrupted
        .entries
        .insert("model".to_owned(), UpdateEntry::Messages(vec![called]));
    plan.passes
        .lock()
        .unwrap()
        .push_back(vec![Ok(update_part(interrupted))]);
    plan.passes
        .lock()
        .unwrap()
        .push_back(vec![Ok(text_part("finished", "model"))]);
    let answers = Arc::new(Mutex::new(VecDeque::from([
        Decision::Allow,
        Decision::Deny {
            reason: "no".to_owned(),
        },
    ])));
    let turn_answers = Arc::clone(&answers);
    let mut handler = handler(runtime);
    let mut ctx = context("/work");
    handler.start(&mut ctx).await.unwrap();
    let events = run_turn(&mut handler, &mut ctx, "do it", move |_| {
        turn_answers.lock().unwrap().pop_front().unwrap()
    })
    .await;
    (plan, events)
}

#[tokio::test]
async fn turn_asks_one_approval_per_action_with_the_normalized_tool_name() {
    let (_plan, events) = run_write_then_bash_turn().await;
    let asked = approvals(&events);
    assert_eq!(asked.len(), 2);
    let mut remaining = asked.into_iter();
    let first = remaining.next().unwrap();
    assert_eq!(first.tool, "write");
    assert_eq!(first.input, json_object(r#"{"path":"a.txt"}"#));
    let second = remaining.next().unwrap();
    assert_eq!(second.tool, "bash");
    assert_eq!(second.input, JsonObject::new());
}

#[tokio::test]
async fn turn_resumes_with_the_decisions_in_order_under_the_interrupt_id() {
    let (plan, events) = run_write_then_bash_turn().await;
    assert_eq!(deltas(&events), ["finished"]);
    let inputs = plan.inputs.lock().unwrap();
    assert_eq!(inputs.len(), 2);
    assert!(matches!(
        inputs.first().unwrap(),
        GraphInput::Messages(message)
            if message.kind == MessageKind::Human && content_text(&message.content) == "do it"
    ));
    assert_eq!(
        inputs.get(1).unwrap(),
        &GraphInput::Resume(BTreeMap::from([(
            "int-1".to_owned(),
            vec![
                HitlDecision::Approve,
                HitlDecision::Reject {
                    message: "no".to_owned()
                }
            ]
        )]))
    );
}

#[tokio::test]
async fn turn_denies_with_the_default_message_when_the_reason_is_empty() {
    let (runtime, plan) = FakeRuntime::new();
    plan.passes
        .lock()
        .unwrap()
        .push_back(vec![Ok(update_part(interrupt_update(
            "int-1",
            &[("write_file", "{}")],
        )))]);
    let mut handler = handler(runtime);
    let mut ctx = context("/work");
    handler.start(&mut ctx).await.unwrap();
    run_turn(&mut handler, &mut ctx, "do it", |_| Decision::Deny {
        reason: String::new(),
    })
    .await;
    let inputs = plan.inputs.lock().unwrap();
    assert_eq!(
        inputs.get(1).unwrap(),
        &GraphInput::Resume(BTreeMap::from([(
            "int-1".to_owned(),
            vec![HitlDecision::Reject {
                message: "The user denied this tool call.".to_owned()
            }]
        )]))
    );
}

#[tokio::test]
async fn turn_repeats_until_a_pass_has_no_interrupts() {
    let (runtime, plan) = FakeRuntime::new();
    plan.passes
        .lock()
        .unwrap()
        .push_back(interrupt_pass("int-1", "write_file"));
    plan.passes
        .lock()
        .unwrap()
        .push_back(interrupt_pass("int-2", "execute"));
    plan.passes.lock().unwrap().push_back(text_pass("done"));
    let mut handler = handler(runtime);
    let mut ctx = context("/work");
    handler.start(&mut ctx).await.unwrap();
    let events = run_turn(&mut handler, &mut ctx, "do it", |_| Decision::Allow).await;
    assert_eq!(approvals(&events).len(), 2);
    assert_eq!(deltas(&events), ["done"]);
    let inputs = plan.inputs.lock().unwrap();
    assert_eq!(inputs.len(), 3);
    assert_eq!(
        inputs.get(1).unwrap(),
        &GraphInput::Resume(BTreeMap::from([(
            "int-1".to_owned(),
            vec![HitlDecision::Approve]
        )]))
    );
    assert_eq!(
        inputs.get(2).unwrap(),
        &GraphInput::Resume(BTreeMap::from([(
            "int-2".to_owned(),
            vec![HitlDecision::Approve]
        )]))
    );
}

#[tokio::test]
async fn turn_collects_every_interrupt_of_a_pass_before_asking() {
    let (runtime, plan) = FakeRuntime::new();
    plan.passes.lock().unwrap().push_back(vec![
        Ok(update_part(interrupt_update(
            "int-1",
            &[("write_file", "{}")],
        ))),
        Ok(text_part("after", "model")),
        Ok(update_part(interrupt_update("int-2", &[("execute", "{}")]))),
    ]);
    plan.passes.lock().unwrap().push_back(text_pass("done"));
    let mut handler = handler(runtime);
    let mut ctx = context("/work");
    handler.start(&mut ctx).await.unwrap();
    let events = run_turn(&mut handler, &mut ctx, "do it", |_| Decision::Allow).await;
    assert_eq!(deltas(&events), ["after", "done"]);
    let text_at = events
        .iter()
        .position(|event| matches!(event, Event::Text { delta } if delta == "after"))
        .unwrap();
    let approval_at: Vec<usize> = events
        .iter()
        .enumerate()
        .filter(|(_, event)| matches!(event, Event::Approval(_)))
        .map(|(index, _)| index)
        .collect();
    assert_eq!(approval_at.len(), 2);
    assert!(approval_at.iter().all(|index| text_at < *index));
    let inputs = plan.inputs.lock().unwrap();
    assert_eq!(inputs.len(), 2);
    assert_eq!(
        inputs.get(1).unwrap(),
        &GraphInput::Resume(BTreeMap::from([
            ("int-1".to_owned(), vec![HitlDecision::Approve]),
            ("int-2".to_owned(), vec![HitlDecision::Approve]),
        ]))
    );
}

#[tokio::test]
async fn turn_never_yields_done() {
    let (runtime, plan) = FakeRuntime::new();
    plan.passes
        .lock()
        .unwrap()
        .push_back(vec![Ok(text_part("done", "model"))]);
    let mut handler = handler(runtime);
    let mut ctx = context("/work");
    handler.start(&mut ctx).await.unwrap();
    let events = run_turn(&mut handler, &mut ctx, "hi", |_| Decision::Allow).await;
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, Event::Done { .. }))
    );
}

#[tokio::test]
async fn turn_fails_with_session_closed_before_start_and_after_stop() {
    let (runtime, _plan) = FakeRuntime::new();
    let mut handler = handler(runtime);
    let mut ctx = context("/work");
    let mut stream = Box::pin(handler.turn(&mut ctx, "hi"));
    let first = stream.next().await.unwrap();
    assert!(matches!(first, Err(HarnessError::SessionClosed)));
    drop(stream);
    handler.start(&mut ctx).await.unwrap();
    handler.stop(&mut ctx).await.unwrap();
    let mut after_stop = Box::pin(handler.turn(&mut ctx, "again"));
    let closed = after_stop.next().await.unwrap();
    assert!(matches!(closed, Err(HarnessError::SessionClosed)));
}

fn one_generation(input_tokens: u64, output_tokens: u64) -> LlmResult {
    let mut called = ai("done");
    called.usage = MessageUsage {
        input_tokens,
        output_tokens,
    };
    LlmResult {
        generations: vec![vec![Generation {
            message: Some(called),
        }]],
    }
}

#[tokio::test]
async fn turn_adds_the_run_usage_to_the_outcome() {
    let (runtime, plan) = FakeRuntime::new();
    plan.passes
        .lock()
        .unwrap()
        .push_back(vec![Ok(text_part("done", "model"))]);
    *plan.usage_per_call.lock().unwrap() = Some(one_generation(10, 5));
    let mut handler = handler(runtime);
    let mut ctx = context("/work");
    handler.start(&mut ctx).await.unwrap();
    run_turn(&mut handler, &mut ctx, "hi", |_| Decision::Allow).await;
    assert_eq!(
        ctx.outcome.usage,
        Usage {
            input_tokens: 10,
            output_tokens: 5,
            calls: 1,
        }
    );
    assert_eq!(ctx.outcome.cost, Decimal::new(15, 0));
}

#[tokio::test]
async fn turn_adds_each_turns_usage_to_the_outcome() {
    let (runtime, plan) = FakeRuntime::new();
    plan.passes
        .lock()
        .unwrap()
        .push_back(vec![Ok(text_part("done", "model"))]);
    plan.passes
        .lock()
        .unwrap()
        .push_back(vec![Ok(text_part("done", "model"))]);
    *plan.usage_per_call.lock().unwrap() = Some(one_generation(10, 5));
    let mut handler = handler(runtime);
    let mut ctx = context("/work");
    handler.start(&mut ctx).await.unwrap();
    run_turn(&mut handler, &mut ctx, "hi", |_| Decision::Allow).await;
    run_turn(&mut handler, &mut ctx, "again", |_| Decision::Allow).await;
    assert_eq!(
        ctx.outcome.usage,
        Usage {
            input_tokens: 20,
            output_tokens: 10,
            calls: 2,
        }
    );
    assert_eq!(ctx.outcome.cost, Decimal::new(30, 0));
}
