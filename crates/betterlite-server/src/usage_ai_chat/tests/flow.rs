#[tokio::test]
async fn flow_streams_a_plain_answer() {
    let parts = chat_parts(fake_model().replying(answer("Hi there.")), fake_source());
    let messages = [ChatMessage::User {
        content: "how much did we spend?".to_owned(),
    }];
    let frames = drive(&parts, &chat_request(&messages, None, Some("alice"), false)).await;
    assert_eq!(
        frames,
        [
            status_frame("Thinking..."),
            chunk_frame("Hi there."),
            done_frame(),
        ]
    );
    let requests = parts.model.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].model, DEFAULT_CHAT_MODEL);
    assert_eq!(
        requests[0].messages,
        [
            ChatMessage::System {
                content: build_system_prompt(false, on(2026, 11, 16))
            },
            messages[0].clone(),
        ]
    );
    assert_eq!(requests[0].tools.as_ref().unwrap().len(), 1);
    assert_eq!(
        requests[0].temperature.to_bits(),
        USAGE_AI_TEMPERATURE.to_bits()
    );
}

#[tokio::test]
async fn flow_streams_tool_events_in_order() {
    let model = fake_model()
        .replying(tool_call("get_usage_data", ARGS))
        .streaming(&["Total", " spend"]);
    let served = page(Vec::new(), metadata("1.5", 1, 1, 0, 10));
    let parts = chat_parts(model, fake_source().serving(served.clone()));
    let messages = [ChatMessage::User {
        content: "how much?".to_owned(),
    }];
    let request = chat_request(&messages, Some(" claude-3 "), Some("alice"), false);
    let days = [("start_date", "2026-01-01"), ("end_date", "2026-01-31")];
    assert_eq!(
        drive(&parts, &request).await,
        [
            status_frame("Thinking..."),
            frame_tool("get_usage_data", "global usage data", &days, "running"),
            frame_tool("get_usage_data", "global usage data", &days, "complete"),
            status_frame("Analyzing results..."),
            chunk_frame("Total"),
            chunk_frame(" spend"),
            done_frame(),
        ]
    );
    assert_eq!(
        parts.data.calls(),
        [SourceCall::Global {
            start_date: "2026-01-01".to_owned(),
            end_date: "2026-01-31".to_owned(),
            user_id: Some("alice".to_owned()),
        }]
    );
    let requests = parts.model.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].model, "claude-3");
    assert_eq!(requests[1].model, "claude-3");
    assert_eq!(requests[1].tools, None);
    assert_eq!(
        requests[1].temperature.to_bits(),
        USAGE_AI_TEMPERATURE.to_bits()
    );
    assert_eq!(requests[1].messages.len(), 4);
    assert_eq!(
        requests[1].messages[2],
        ChatMessage::AssistantToolCalls {
            content: None,
            tool_calls: vec![ModelToolCall {
                id: "call-1".to_owned(),
                name: "get_usage_data".to_owned(),
                arguments: ARGS.to_owned(),
            }],
        }
    );
    assert_eq!(
        requests[1].messages[3],
        ChatMessage::Tool {
            tool_call_id: "call-1".to_owned(),
            content: summarise_usage_data(&served).unwrap(),
        }
    );
}

#[tokio::test]
async fn flow_drops_text_that_arrives_with_tool_calls() {
    let reply = ModelReply {
        content: Some("ignored text".to_owned()),
        tool_calls: vec![ModelToolCall {
            id: "call-1".to_owned(),
            name: "get_usage_data".to_owned(),
            arguments: ARGS.to_owned(),
        }],
    };
    let parts = chat_parts(
        fake_model().replying(reply).streaming(&["answer"]),
        fake_source().serving(empty_page()),
    );
    let messages = [ChatMessage::User {
        content: "hi".to_owned(),
    }];
    let frames = drive(&parts, &chat_request(&messages, None, Some("alice"), false)).await;
    assert!(!frames.iter().any(|frame| frame.contains("ignored text")));
    assert_eq!(frames.last().unwrap(), &done_frame());
    assert_eq!(
        parts.model.requests()[1].messages[2],
        ChatMessage::AssistantToolCalls {
            content: Some("ignored text".to_owned()),
            tool_calls: vec![ModelToolCall {
                id: "call-1".to_owned(),
                name: "get_usage_data".to_owned(),
                arguments: ARGS.to_owned(),
            }],
        }
    );
}

#[tokio::test]
async fn flow_runs_every_admin_tool_call_in_order() {
    let reply = ModelReply {
        content: None,
        tool_calls: vec![
            ModelToolCall {
                id: "call-1".to_owned(),
                name: "get_team_usage_data".to_owned(),
                arguments: "{\"start_date\":\"2026-01-01\",\"end_date\":\"2026-01-31\",\"team_ids\":\"team-1, team-2\"}".to_owned(),
            },
            ModelToolCall {
                id: "call-2".to_owned(),
                name: "get_tag_usage_data".to_owned(),
                arguments: "{\"start_date\":\"2026-01-01\",\"end_date\":\"2026-01-31\"}".to_owned(),
            },
        ],
    };
    let team_page = page(Vec::new(), metadata("2", 1, 1, 0, 5));
    let tag_page = empty_page();
    let parts = chat_parts(
        fake_model().replying(reply).streaming(&["done"]),
        fake_source()
            .serving(team_page.clone())
            .serving(tag_page.clone()),
    );
    let messages = [ChatMessage::User {
        content: "teams and tags".to_owned(),
    }];
    let request = chat_request(&messages, None, Some("admin-1"), true);
    let frames = drive(&parts, &request).await;
    let id_days = [("start_date", "2026-01-01"), ("end_date", "2026-01-31")];
    assert_eq!(frames.len(), 8);
    assert_eq!(
        frames[1],
        frame_tool(
            "get_team_usage_data",
            "team usage data",
            &id_days,
            "running"
        )
    );
    assert_eq!(
        frames[2],
        frame_tool(
            "get_team_usage_data",
            "team usage data",
            &id_days,
            "complete"
        )
    );
    assert_eq!(
        frames[3],
        frame_tool("get_tag_usage_data", "tag usage data", &id_days, "running")
    );
    assert_eq!(
        frames[4],
        frame_tool("get_tag_usage_data", "tag usage data", &id_days, "complete")
    );
    assert_eq!(frames[7], done_frame());
    assert_eq!(
        parts.data.calls(),
        [
            SourceCall::Teams {
                start_date: "2026-01-01".to_owned(),
                end_date: "2026-01-31".to_owned(),
                team_ids: Some(vec!["team-1".to_owned(), "team-2".to_owned()]),
            },
            SourceCall::Tags {
                start_date: "2026-01-01".to_owned(),
                end_date: "2026-01-31".to_owned(),
                tags: None,
            },
        ]
    );
    let requests = parts.model.requests();
    assert_eq!(requests[0].tools.as_ref().unwrap().len(), 3);
    assert_eq!(
        requests[1].messages[3],
        ChatMessage::Tool {
            tool_call_id: "call-1".to_owned(),
            content: summarise_entity_data(&team_page, EntityLabel::Team).unwrap(),
        }
    );
    assert_eq!(
        requests[1].messages[4],
        ChatMessage::Tool {
            tool_call_id: "call-2".to_owned(),
            content: summarise_entity_data(&tag_page, EntityLabel::Tag).unwrap(),
        }
    );
}

#[tokio::test]
async fn flow_reports_a_tool_failure_and_keeps_going() {
    let model = fake_model()
        .replying(tool_call("get_usage_data", ARGS))
        .streaming(&["Sorry."]);
    let parts = chat_parts(model, fake_source().failing(UsageChatError::Overflow));
    let messages = [ChatMessage::User {
        content: "hi".to_owned(),
    }];
    let frames = drive(&parts, &chat_request(&messages, None, Some("alice"), false)).await;
    let days = [("start_date", "2026-01-01"), ("end_date", "2026-01-31")];
    assert_eq!(
        frames,
        [
            status_frame("Thinking..."),
            frame_tool("get_usage_data", "global usage data", &days, "running"),
            frame_tool("get_usage_data", "global usage data", &days, "error"),
            status_frame("Analyzing results..."),
            chunk_frame("Sorry."),
            done_frame(),
        ]
    );
    assert_eq!(
        parts.model.requests()[1].messages[3],
        ChatMessage::Tool {
            tool_call_id: "call-1".to_owned(),
            content: "Error fetching global usage data. Please try again.".to_owned(),
        }
    );
}

#[tokio::test]
async fn flow_turns_an_unavailable_tool_into_a_tool_message() {
    let model = fake_model()
        .replying(tool_call("get_team_usage_data", ARGS))
        .streaming(&["ok"]);
    let parts = chat_parts(model, fake_source());
    let messages = [ChatMessage::User {
        content: "team spend?".to_owned(),
    }];
    let frames = drive(&parts, &chat_request(&messages, None, Some("alice"), false)).await;
    assert_eq!(
        frames,
        [
            status_frame("Thinking..."),
            status_frame("Analyzing results..."),
            chunk_frame("ok"),
            done_frame(),
        ]
    );
    assert!(parts.data.calls().is_empty());
    assert_eq!(
        parts.model.requests()[1].messages[3],
        ChatMessage::Tool {
            tool_call_id: "call-1".to_owned(),
            content: "Tool not available: get_team_usage_data".to_owned(),
        }
    );
}

#[tokio::test]
async fn flow_ends_with_a_generic_error_on_bad_tool_arguments() {
    let parts = chat_parts(
        fake_model().replying(tool_call("get_usage_data", "not json")),
        fake_source(),
    );
    let messages = [ChatMessage::User {
        content: "hi".to_owned(),
    }];
    assert_eq!(
        drive(&parts, &chat_request(&messages, None, Some("alice"), false)).await,
        [status_frame("Thinking..."), ERROR_FRAME.to_owned()]
    );
    assert!(parts.data.calls().is_empty());
    assert_eq!(parts.model.requests().len(), 1);
}

#[tokio::test]
async fn flow_ends_with_a_generic_error_when_the_model_fails() {
    let parts = chat_parts(fake_model().failing_completion("boom"), fake_source());
    let messages = [ChatMessage::User {
        content: "hi".to_owned(),
    }];
    assert_eq!(
        drive(&parts, &chat_request(&messages, None, Some("alice"), false)).await,
        [status_frame("Thinking..."), ERROR_FRAME.to_owned()]
    );
}

#[tokio::test]
async fn flow_ends_with_a_generic_error_when_the_stream_fails() {
    let model = fake_model()
        .replying(tool_call("get_usage_data", ARGS))
        .failing_stream("down");
    let parts = chat_parts(model, fake_source().serving(empty_page()));
    let messages = [ChatMessage::User {
        content: "hi".to_owned(),
    }];
    let frames = drive(&parts, &chat_request(&messages, None, Some("alice"), false)).await;
    let days = [("start_date", "2026-01-01"), ("end_date", "2026-01-31")];
    assert_eq!(
        frames,
        [
            status_frame("Thinking..."),
            frame_tool("get_usage_data", "global usage data", &days, "running"),
            frame_tool("get_usage_data", "global usage data", &days, "complete"),
            status_frame("Analyzing results..."),
            ERROR_FRAME.to_owned(),
        ]
    );
}

#[tokio::test]
async fn flow_keeps_only_the_last_twenty_messages() {
    let messages: Vec<ChatMessage> = (0..25_usize)
        .map(|index| ChatMessage::User {
            content: format!("message {index}"),
        })
        .collect();
    let parts = chat_parts(fake_model().replying(answer("ok")), fake_source());
    drive(&parts, &chat_request(&messages, None, Some("alice"), false)).await;
    let recorded = &parts.model.requests()[0].messages;
    assert_eq!(recorded.len(), 21);
    assert_eq!(
        recorded[0],
        ChatMessage::System {
            content: build_system_prompt(false, on(2026, 11, 16))
        }
    );
    assert_eq!(
        recorded[1],
        ChatMessage::User {
            content: "message 5".to_owned()
        }
    );
    assert_eq!(
        recorded[20],
        ChatMessage::User {
            content: "message 24".to_owned()
        }
    );
}

#[tokio::test]
async fn flow_falls_back_to_the_default_model_and_trims_it() {
    let messages = [ChatMessage::User {
        content: "hi".to_owned(),
    }];
    let blank = chat_parts(fake_model().replying(answer("a")), fake_source());
    drive(&blank, &chat_request(&messages, Some("   "), None, false)).await;
    assert_eq!(blank.model.requests()[0].model, DEFAULT_CHAT_MODEL);
    let spaced = chat_parts(fake_model().replying(answer("b")), fake_source());
    drive(
        &spaced,
        &chat_request(&messages, Some("  claude-3  "), None, true),
    )
    .await;
    assert_eq!(spaced.model.requests()[0].model, "claude-3");
}

#[tokio::test]
async fn flow_skips_an_empty_plain_answer() {
    let model = fake_model().replying(ModelReply {
        content: Some(String::new()),
        tool_calls: Vec::new(),
    });
    let parts = chat_parts(model, fake_source());
    let messages = [ChatMessage::User {
        content: "hi".to_owned(),
    }];
    assert_eq!(
        drive(&parts, &chat_request(&messages, None, Some("alice"), false)).await,
        [status_frame("Thinking..."), done_frame()]
    );
}

#[tokio::test]
async fn flow_skips_an_empty_stream_delta() {
    let model = fake_model()
        .replying(tool_call("get_usage_data", ARGS))
        .streaming(&["", "x"]);
    let parts = chat_parts(model, fake_source().serving(empty_page()));
    let messages = [ChatMessage::User {
        content: "hi".to_owned(),
    }];
    let frames = drive(&parts, &chat_request(&messages, None, Some("alice"), false)).await;
    assert_eq!(frames.last().unwrap(), &done_frame());
    assert_eq!(
        frames
            .iter()
            .filter(|frame| frame.contains("chunk"))
            .count(),
        1
    );
    assert_eq!(frames[4], chunk_frame("x"));
}

#[tokio::test]
async fn flow_replaces_the_models_user_id_for_a_non_admin() {
    let arguments =
        "{\"start_date\":\"2026-01-01\",\"end_date\":\"2026-01-31\",\"user_id\":\"mallory\"}";
    let model = fake_model()
        .replying(tool_call("get_usage_data", arguments))
        .streaming(&["ok"]);
    let parts = chat_parts(model, fake_source().serving(empty_page()));
    let messages = [ChatMessage::User {
        content: "hi".to_owned(),
    }];
    let frames = drive(&parts, &chat_request(&messages, None, Some("alice"), false)).await;
    assert!(frames[1].contains("\"user_id\": \"mallory\""));
    assert_eq!(
        parts.data.calls(),
        [SourceCall::Global {
            start_date: "2026-01-01".to_owned(),
            end_date: "2026-01-31".to_owned(),
            user_id: Some("alice".to_owned()),
        }]
    );
}

#[tokio::test]
async fn flow_stops_quietly_when_the_client_left() {
    let parts = chat_parts(fake_model().replying(answer("hi")), fake_source());
    let messages = [ChatMessage::User {
        content: "hi".to_owned(),
    }];
    let (sink, receiver) = mpsc::channel(4);
    drop(receiver);
    stream_usage_ai_chat(
        &parts,
        &chat_request(&messages, None, Some("alice"), false),
        &sink,
    )
    .await;
    assert!(parts.model.requests().is_empty());
}

#[test]
fn flow_future_is_send() {
    let parts = chat_parts(fake_model(), fake_source());
    let messages = [ChatMessage::User {
        content: "hi".to_owned(),
    }];
    let (sink, _receiver) = mpsc::channel(4);
    drop(require_send(stream_usage_ai_chat(
        &parts,
        &chat_request(&messages, None, None, false),
        &sink,
    )));
}
