use betterlite_mcp::openapi_tools::output::OpenApiToolOutput;
use betterlite_mcp::tool_registry::handler::ToolHandlerError;

use super::support::{call, fixture, load_tool, prompt_json};

#[tokio::test]
async fn passes_the_arguments_as_keywords() {
    let fixture = fixture();
    let source = "def probe(*args, **kwargs):\n    return {\"positional\": len(args), \"keywords\": kwargs}\n";
    load_tool(&fixture, "b8871_probe", source, "probe").await;
    let output = call(&fixture, "tool", r#"{"city": "Paris", "unit": "c"}"#)
        .await
        .unwrap();
    let OpenApiToolOutput::Json {
        value,
        original_text,
    } = output
    else {
        panic!("expected a structured result, got a text one");
    };
    assert_eq!(
        original_text,
        r#"{"keywords":{"city":"Paris","unit":"c"},"positional":0}"#
    );
    assert_eq!(
        value,
        prompt_json(r#"{"keywords": {"city": "Paris", "unit": "c"}, "positional": 0}"#)
    );
}

#[tokio::test]
async fn calls_an_async_handler() {
    let fixture = fixture();
    let source = "async def echo(city):\n    return \"async \" + city\n";
    load_tool(&fixture, "b8871_async", source, "echo").await;
    assert_eq!(
        call(&fixture, "tool", r#"{"city": "Paris"}"#)
            .await
            .unwrap(),
        OpenApiToolOutput::Text("async Paris".to_owned())
    );
}

#[tokio::test]
async fn passes_a_nested_argument_unchanged() {
    let fixture = fixture();
    let source = "def echo(**kwargs):\n    return kwargs\n";
    load_tool(&fixture, "b8871_nested_args", source, "echo").await;
    let arguments = r#"{"options": {"limit": 2, "tags": ["a", null, true]}}"#;
    let output = call(&fixture, "tool", arguments).await.unwrap();
    let OpenApiToolOutput::Json {
        value,
        original_text,
    } = output
    else {
        panic!("expected a structured result, got a text one");
    };
    assert_eq!(
        original_text,
        r#"{"options":{"limit":2,"tags":["a",null,true]}}"#
    );
    assert_eq!(
        value,
        prompt_json(r#"{"options": {"limit": 2, "tags": ["a", null, true]}}"#)
    );
}

#[tokio::test]
async fn rejects_an_unknown_keyword() {
    let fixture = fixture();
    let source = "def takes_city(city):\n    return city\n";
    load_tool(&fixture, "b8871_unknown", source, "takes_city").await;
    match call(&fixture, "tool", r#"{"extra": 1}"#).await {
        Err(ToolHandlerError::Failed(message)) => {
            assert!(
                message.contains("unexpected keyword argument 'extra'"),
                "message was {message}"
            );
        }
        other => panic!("expected the call to fail, got {other:?}"),
    }
}

#[tokio::test]
async fn rejects_a_missing_keyword() {
    let fixture = fixture();
    let source = "def takes_city(city):\n    return city\n";
    load_tool(&fixture, "b8871_missing", source, "takes_city").await;
    match call(&fixture, "tool", "{}").await {
        Err(ToolHandlerError::Failed(message)) => {
            assert!(
                message.contains("missing 1 required positional argument: 'city'"),
                "message was {message}"
            );
        }
        other => panic!("expected the call to fail, got {other:?}"),
    }
}

#[tokio::test]
async fn keeps_a_returned_string() {
    let fixture = fixture();
    let source = "def reply(**kwargs):\n    return \"hello\"\n";
    load_tool(&fixture, "b8871_hello", source, "reply").await;
    assert_eq!(
        call(&fixture, "tool", "{}").await.unwrap(),
        OpenApiToolOutput::Text("hello".to_owned())
    );
}

#[tokio::test]
async fn keeps_an_empty_returned_string() {
    let fixture = fixture();
    let source = "def reply(**kwargs):\n    return \"\"\n";
    load_tool(&fixture, "b8871_empty", source, "reply").await;
    assert_eq!(
        call(&fixture, "tool", "{}").await.unwrap(),
        OpenApiToolOutput::Text(String::new())
    );
}

#[tokio::test]
async fn keeps_a_returned_object() {
    let fixture = fixture();
    let source = "def reply(**kwargs):\n    return {\"a\": 1, \"b\": [True, None]}\n";
    load_tool(&fixture, "b8871_object", source, "reply").await;
    let output = call(&fixture, "tool", "{}").await.unwrap();
    let OpenApiToolOutput::Json {
        value,
        original_text,
    } = output
    else {
        panic!("expected a structured result, got a text one");
    };
    assert_eq!(original_text, r#"{"a":1,"b":[true,null]}"#);
    assert_eq!(value, prompt_json(r#"{"a": 1, "b": [true, null]}"#));
}

#[tokio::test]
async fn keeps_a_returned_array() {
    let fixture = fixture();
    let source = "def reply(**kwargs):\n    return [1, \"two\"]\n";
    load_tool(&fixture, "b8871_array", source, "reply").await;
    let output = call(&fixture, "tool", "{}").await.unwrap();
    let OpenApiToolOutput::Json {
        value,
        original_text,
    } = output
    else {
        panic!("expected a structured result, got a text one");
    };
    assert_eq!(original_text, r#"[1,"two"]"#);
    assert_eq!(value, prompt_json(r#"[1, "two"]"#));
}

#[tokio::test]
async fn keeps_a_returned_null() {
    let fixture = fixture();
    let source = "def reply(**kwargs):\n    return None\n";
    load_tool(&fixture, "b8871_null", source, "reply").await;
    let output = call(&fixture, "tool", "{}").await.unwrap();
    let OpenApiToolOutput::Json {
        value,
        original_text,
    } = output
    else {
        panic!("expected a structured result, got a text one");
    };
    assert_eq!(original_text, "null");
    assert_eq!(value, prompt_json("null"));
}

#[tokio::test]
async fn keeps_a_raised_exception_message() {
    let fixture = fixture();
    let source = "def boom(**kwargs):\n    raise ValueError(\"the city is unknown\")\n";
    load_tool(&fixture, "b8871_boom", source, "boom").await;
    match call(&fixture, "tool", "{}").await {
        Err(ToolHandlerError::Failed(message)) => assert_eq!(message, "the city is unknown"),
        other => panic!("expected the call to fail, got {other:?}"),
    }
}

#[tokio::test]
async fn fails_when_the_attribute_is_not_callable() {
    let fixture = fixture();
    load_tool(&fixture, "b8871_answer", "answer = 42\n", "answer").await;
    match call(&fixture, "tool", "{}").await {
        Err(ToolHandlerError::Failed(message)) => {
            assert!(
                message.contains("'int' object is not callable"),
                "message was {message}"
            );
        }
        other => panic!("expected the call to fail, got {other:?}"),
    }
}

#[tokio::test]
async fn fails_when_the_return_value_has_no_json_form() {
    let fixture = fixture();
    let source = "def thing(**kwargs):\n    return object()\n";
    load_tool(&fixture, "b8871_thing", source, "thing").await;
    match call(&fixture, "tool", "{}").await {
        Err(ToolHandlerError::Failed(message)) => {
            assert_eq!(message, "a Python object value has no JSON form");
        }
        other => panic!("expected the call to fail, got {other:?}"),
    }
}
