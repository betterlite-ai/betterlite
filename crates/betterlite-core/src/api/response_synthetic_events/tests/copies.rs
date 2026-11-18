#![cfg(test)]

use super::*;

#[test]
fn logging_copy_returns_nonmodel_events_unchanged() {
    let value = json(r#"{"type":"response.output_text.delta","delta":"x"}"#);
    match logging_copy(LoggingEvent::Value(&value)) {
        LoggedCopy::Same(same) => assert!(std::ptr::eq(same, &value)),
        LoggedCopy::Detached(_) => panic!("plain JSON is handed back unchanged"),
    }
    let generic = ResponsesAPIStreamingResponse::Generic(GenericEvent {
        r#type: "response.provider.event".to_owned(),
        extra: BTreeMap::new(),
    });
    match logging_copy(LoggingEvent::Event(&generic)) {
        LoggedCopy::Detached(copied) => assert_eq!(
            json_value::to_json(copied.as_ref()).unwrap(),
            json(r#"{"type":"response.provider.event"}"#)
        ),
        LoggedCopy::Same(_) => panic!("events are detached from the caller"),
    }
}

#[test]
fn logging_copy_round_trips_models_and_detaches_usage() {
    let output = format!("[{}]", message_with_text("msg_a", "hi"));
    let mut transformed = response_with(
        &output,
        r#","usage":{"input_tokens":2,"output_tokens":3,"total_tokens":5}"#,
    );
    let events = build_synthetic_response_events(&mut transformed, None::<&FixedCost>, 5).unwrap();
    let completed = events.last().unwrap();
    let before = json_value::to_json(completed).unwrap();
    let LoggedCopy::Detached(mut copied) = logging_copy(LoggingEvent::Event(completed)) else {
        panic!("model events are copied");
    };
    assert_eq!(json_value::to_json(copied.as_ref()).unwrap(), before);
    let ResponsesAPIStreamingResponse::Known(known) = copied.as_mut() else {
        panic!("known event stays known");
    };
    let KnownResponseEvent::ResponseCompleted(event) = known.as_mut() else {
        panic!("completed event stays completed");
    };
    event.response.usage.as_mut().unwrap().cost = Some(Decimal::new(999, 2));
    let ResponsesAPIStreamingResponse::Known(original) = completed else {
        panic!("original stays known");
    };
    let KnownResponseEvent::ResponseCompleted(original) = original.as_ref() else {
        panic!("original stays completed");
    };
    assert!(original.response.usage.as_ref().unwrap().cost.is_none());
    assert_eq!(text(at(&before, &["response", "status"])), "completed");
    assert_eq!(
        at(&before, &["response", "usage", "cost"]),
        &Json::Null,
        "the original usage carries no cost"
    );
}

#[test]
fn logging_copy_falls_back_when_validation_fails() {
    let mut extra = Extra::new();
    extra.insert("delta".to_owned(), json("5"));
    let event = ResponsesAPIStreamingResponse::Known(Box::new(
        KnownResponseEvent::OutputTextDelta(OutputTextDeltaEvent {
            item_id: "item_1".to_owned(),
            output_index: 0,
            content_index: 0,
            delta: "x".to_owned(),
            extra,
        }),
    ));
    let ResponsesAPIStreamingResponse::Known(known) = &event else {
        panic!("the constructed event is known");
    };
    assert!(super::super::copies::round_trip(known.as_ref()).is_none());
    let LoggedCopy::Detached(mut copied) = logging_copy(LoggingEvent::Event(&event)) else {
        panic!("events are copied");
    };
    if let ResponsesAPIStreamingResponse::Known(known) = copied.as_mut() {
        if let KnownResponseEvent::OutputTextDelta(delta) = known.as_mut() {
            delta.item_id = "mutated".to_owned();
        }
    }
    assert_eq!(
        json_value::to_json(&event).unwrap(),
        json(
            r#"{"type":"response.output_text.delta","item_id":"item_1","output_index":0,"content_index":0,"delta":5}"#
        )
    );
    assert_eq!(
        json_value::to_json(copied.as_ref()).unwrap(),
        json(
            r#"{"type":"response.output_text.delta","item_id":"mutated","output_index":0,"content_index":0,"delta":5}"#
        )
    );
}

#[test]
fn usage_as_model_handles_models_dicts_and_malformed_values() {
    let model = usage(1, 2);
    assert_eq!(
        usage_as_model(Some(UsageSource::Model(&model))),
        Some(model.clone())
    );
    assert_eq!(
        usage_as_model(Some(UsageSource::Value(&json(
            r#"{"input_tokens":1,"output_tokens":2,"total_tokens":3,"cost":0.4}"#
        )))),
        Some(ResponseApiUsage {
            cost: Some(Decimal::new(4, 1)),
            ..usage(1, 2)
        })
    );
    let with_extras = usage_as_model(Some(UsageSource::Value(&json(
        r#"{"input_tokens":1,"output_tokens":2,"total_tokens":3,"extra":"kept"}"#,
    ))))
    .unwrap();
    assert_eq!(
        with_extras.extra.get("extra"),
        Some(&Json::Text("kept".to_owned()))
    );
    for malformed in [
        json(r#"{"input_tokens":1}"#),
        json("{}"),
        json("null"),
        json("[1,2]"),
        json(r#""x""#),
        json("true"),
    ] {
        assert!(usage_as_model(Some(UsageSource::Value(&malformed))).is_none());
    }
    assert!(usage_as_model(None).is_none());
}

#[test]
fn dump_json_safely_serializes_responses() {
    let output = format!("[{}]", message_with_text("msg_a", "hi"));
    let transformed = response_with(
        &output,
        r#","usage":{"input_tokens":2,"output_tokens":3,"total_tokens":5,"cost":0.5}"#,
    );
    let dumped = dump_json_safely(&transformed).unwrap();
    let value = json(&dumped);
    assert_eq!(text(field(&value, "id")), "resp_a");
    assert_eq!(number_text(at(&value, &["usage", "cost"])), "0.5");
    let message = item_at(field(&value, "output"), 0);
    assert_eq!(
        text(field(item_at(field(message, "content"), 0), "text")),
        "hi"
    );
}

#[test]
fn dump_json_safely_returns_none_and_logs_on_failure() {
    let (dumped, logs) = capture(|| dump_json_safely(&BrokenResponse));
    assert!(dumped.is_none());
    assert_eq!(logs.len(), 1);
    assert!(logs[0].starts_with("could not serialize completed response for cache: "));
}

#[test]
fn reasoning_item_dump_restores_null_fields() {
    let output = concat!(
        r#"[{"type":"reasoning","id":"rs_1","summary":[{"type":"summary_text","text":"think"}],"#,
        r#""encrypted_content":"enc","status":"completed"}]"#
    );
    let mut transformed = response(output);
    let items = build(&mut transformed, None, 5);
    let added_item = at(event_of(&items, "response.output_item.added"), &["item"]);
    assert_eq!(text(field(added_item, "id")), "rs_1");
    assert_eq!(field(added_item, "content"), &json("null"));
    assert_eq!(text(field(added_item, "encrypted_content")), "enc");
    assert_eq!(text(field(added_item, "status")), "completed");
    let completed_item = item_at(at(items.last().unwrap(), &["response", "output"]), 0);
    assert!(!has_key(completed_item, "content"));
    assert_eq!(text(field(completed_item, "encrypted_content")), "enc");
    let bare = response(r#"[{"type":"reasoning","id":"rs_2","summary":[]}]"#);
    let typed = &bare.output[0];
    let dumped = dump_response_object(ResponseObject::Model(typed)).unwrap();
    let dumped_json = Json::Object(dumped.into_iter().collect());
    assert_eq!(
        dumped_json,
        json(
            r#"{"type":"reasoning","id":"rs_2","summary":[],"content":null,"encrypted_content":null,"status":null}"#
        )
    );
}

#[test]
fn custom_call_dump_keeps_null_caller_and_namespace() {
    let output = concat!(
        r#"[{"type":"message","id":"msg_a","status":"completed","role":"assistant","content":[{"type":"output_text","text":"hi","annotations":[],"logprobs":null}]},"#,
        r#"{"type":"custom_tool_call","id":"ct_1","call_id":"call_1","name":"g","input":"raw","status":"completed"},"#,
        r#"{"type":"custom_tool_call","id":"ct_2","call_id":"call_2","name":"g","input":"raw","caller":{"type":"direct"},"namespace":"ns"}]"#
    );
    let transformed = response(output);
    let bare = dump_response_object(ResponseObject::Model(&transformed.output[1])).unwrap();
    let bare_json = Json::Object(bare.into_iter().collect());
    assert_eq!(
        bare_json,
        json(
            r#"{"type":"custom_tool_call","call_id":"call_1","input":"raw","name":"g","id":"ct_1","caller":null,"namespace":null,"status":"completed"}"#
        )
    );
    let called = dump_response_object(ResponseObject::Model(&transformed.output[2])).unwrap();
    let called_json = Json::Object(called.into_iter().collect());
    assert_eq!(
        called_json,
        json(
            r#"{"type":"custom_tool_call","call_id":"call_2","input":"raw","name":"g","id":"ct_2","caller":{"type":"direct"},"namespace":"ns"}"#
        )
    );
}

#[test]
fn dump_response_object_handles_models_objects_and_scalars() {
    let object = json(r#"{"type":"mystery","payload":1}"#);
    assert_eq!(
        dump_response_object(ResponseObject::Value(&object)).unwrap(),
        match &object {
            Json::Object(fields) => fields.clone().into_iter().collect(),
            _ => Extra::new(),
        }
    );
    for scalar in [json("7"), json(r#""x""#), json("null"), json("[1]")] {
        assert!(
            dump_response_object(ResponseObject::Value(&scalar))
                .unwrap()
                .is_empty()
        );
    }
    let transformed = response(r#"[{"type":"mystery","id":"m","payload":1}]"#);
    assert_eq!(
        Json::Object(
            dump_response_object(ResponseObject::Model(&transformed.output[0]))
                .unwrap()
                .into_iter()
                .collect()
        ),
        json(r#"{"type":"mystery","id":"m","payload":1}"#)
    );
}

#[test]
fn python_value_semantics_match_litellm() {
    assert!(!truthy(&json("null")));
    assert!(truthy(&json("true")));
    assert!(!truthy(&json("false")));
    assert!(!truthy(&json("0")));
    assert!(!truthy(&json("0.0")));
    assert!(truthy(&json("1")));
    assert!(!truthy(&json(r#""""#)));
    assert!(truthy(&json(r#""x""#)));
    assert!(!truthy(&json("[]")));
    assert!(truthy(&json("[0]")));
    assert!(!truthy(&json("{}")));
    assert!(truthy(&json(r#"{"a":0}"#)));
    assert_eq!(text_or_empty(None), "");
    assert_eq!(text_or_empty(Some(&json("null"))), "");
    assert_eq!(text_or_empty(Some(&json("false"))), "");
    assert_eq!(text_or_empty(Some(&json(r#""""#))), "");
    assert_eq!(text_or_empty(Some(&json("0"))), "");
    assert_eq!(text_or_empty(Some(&json("[]"))), "");
    assert_eq!(text_or_empty(Some(&json("{}"))), "");
    assert_eq!(text_or_empty(Some(&json("true"))), "True");
    assert_eq!(text_or_empty(Some(&json("7"))), "7");
    assert_eq!(text_or_empty(Some(&json(r#""x""#))), "x");
    assert_eq!(text_or_empty(Some(&json("[null]"))), "[None]");
}

#[test]
fn tool_arguments_delta_events_cover_three_types() {
    assert_eq!(
        TOOL_ARGUMENTS_DELTA_EVENTS,
        [
            "response.custom_tool_call_input.delta",
            "response.function_call_arguments.delta",
            "response.mcp_call_arguments.delta",
        ]
    );
}
