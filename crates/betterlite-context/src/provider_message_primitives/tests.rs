#![cfg(test)]

use std::fmt;
use std::sync::{Arc, Mutex};

use tracing::field::Field as EventField;
use tracing::subscriber::with_default;
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use uuid::Uuid;

use super::{
    BatchConfigBase, ChatCompletionId, ChoiceFunction, ChoicePolicy, FormatParams, FormatSupport,
    FunctionChoice, FunctionKind, FunctionTool, NestedSchema, ProviderMessage, Record,
    ResponseFormat, Schema, StreamState, SystemRole, ToolCallProjection, ToolChoice, ToolFunction,
    ToolParam, add_response_format, developer_to_system, project_tool_call,
};
use crate::error::ContextError;

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<String>>>);

impl<Inner: Subscriber> Layer<Inner> for Captured {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        let mut message = String::new();
        event.record(&mut |field: &EventField, value: &dyn fmt::Debug| {
            message = format!("{} {value:?}", field.name());
        });
        let line = format!("{} {message}", event.metadata().level());
        self.0.lock().unwrap().push(line);
    }
}

fn capture<Output, Call>(call: Call) -> (Output, Vec<String>)
where
    Call: FnOnce() -> Output,
{
    let captured = Captured::default();
    let subscriber = tracing_subscriber::registry().with(captured.clone());
    let output = with_default(subscriber, call);
    let lines = captured.0.lock().unwrap().clone();
    (output, lines)
}

fn record(fields: &[(&str, Option<&str>)]) -> Record<String> {
    fields
        .iter()
        .map(|(key, field)| ((*key).to_owned(), field.map(str::to_owned)))
        .collect()
}

fn params() -> FormatParams<String> {
    FormatParams {
        tools: Some(vec![ToolParam::Supplied("weather".to_owned())]),
        tool_choice: Some(ToolChoice::Supplied("auto".to_owned())),
        json_mode: None,
        response_format: Some("earlier".to_owned()),
    }
}

fn format(
    response_schema: Option<Schema<String>>,
    json_schema: Option<NestedSchema<String>>,
) -> ResponseFormat<String> {
    ResponseFormat {
        value: "format".to_owned(),
        response_schema,
        json_schema,
    }
}

fn json_tool(parameters: &str) -> ToolParam<String> {
    ToolParam::ResponseFormat(FunctionTool {
        kind: FunctionKind::Function,
        function: ToolFunction {
            name: "json_tool_call",
            parameters: parameters.to_owned(),
        },
    })
}

fn apply(
    format: ResponseFormat<String>,
    support: FormatSupport,
    policy: ChoicePolicy,
) -> (FormatParams<String>, Result<(), ContextError>) {
    let mut updated = params();
    let (result, logs) = capture(|| add_response_format(&mut updated, format, support, policy));
    assert!(logs.is_empty());
    (updated, result)
}

fn native(response_schema: Option<Schema<String>>, json_schema: Option<NestedSchema<String>>) {
    let native_format = format(response_schema, json_schema);
    let (updated, result) = apply(native_format, FormatSupport::Native, ChoicePolicy::Enforce);
    result.unwrap();
    let assigned = FormatParams {
        response_format: Some("format".to_owned()),
        ..params()
    };
    assert_eq!(updated, assigned);
}

fn malformed(json_schema: NestedSchema<String>, error: ContextError) {
    let nested = format(None, Some(json_schema));
    let (updated, result) = apply(nested, FormatSupport::Native, ChoicePolicy::Enforce);
    assert_eq!(updated, params());
    assert_eq!(result, Err(error));
}

fn response_format_native() {
    native(None, None);
    native(Some(Schema::Falsy), None);
    native(None, Some(NestedSchema::Object(Some(Schema::Falsy))));
    native(
        Some(Schema::Truthy("top".to_owned())),
        Some(NestedSchema::NotObject),
    );
    malformed(NestedSchema::Object(None), ContextError::MissingSchema);
    malformed(NestedSchema::NotObject, ContextError::SchemaNotObject);
}

fn emulated(policy: ChoicePolicy, schema: &str) -> FormatParams<String> {
    let nested = Some(NestedSchema::Object(Some(Schema::Truthy(
        schema.to_owned(),
    ))));
    let (updated, result) = apply(format(None, nested), FormatSupport::ToolCall, policy);
    result.unwrap();
    updated
}

fn response_format_tool() {
    let choice = ToolChoice::ResponseFormat(FunctionChoice {
        kind: FunctionKind::Function,
        function: ChoiceFunction {
            name: "json_tool_call",
        },
    });
    let enforced = FormatParams {
        tools: Some(vec![
            ToolParam::Supplied("weather".to_owned()),
            json_tool("schema"),
        ]),
        tool_choice: Some(choice),
        json_mode: Some(true),
        response_format: Some("earlier".to_owned()),
    };
    assert_eq!(emulated(ChoicePolicy::Enforce, "schema"), enforced);
    let kept = emulated(ChoicePolicy::KeepExisting, "schema");
    assert_eq!(kept.tool_choice, params().tool_choice);
    let mut empty = FormatParams {
        tools: None,
        tool_choice: None,
        json_mode: None,
        response_format: None,
    };
    let top = format(Some(Schema::Truthy("top".to_owned())), None);
    add_response_format(
        &mut empty,
        top,
        FormatSupport::ToolCall,
        ChoicePolicy::Enforce,
    )
    .unwrap();
    assert_eq!(empty.tools, Some(vec![json_tool("top")]));
    let falsy = format(Some(Schema::Falsy), None);
    let (unchanged, result) = apply(falsy, FormatSupport::ToolCall, ChoicePolicy::Enforce);
    result.unwrap();
    assert_eq!(unchanged, params());
}

fn developer_role_projection() {
    let messages = vec![
        record(&[("role", Some("user")), ("content", Some("hi"))]),
        record(&[
            ("role", Some("developer")),
            ("content", Some("be brief")),
            ("name", Some("ops")),
        ]),
        record(&[("role", None), ("content", Some("odd"))]),
        record(&[("role", Some("developer")), ("content", None)]),
    ];
    let (projected, logs) = capture(|| developer_to_system(messages.clone()));
    let system = |content: Option<&str>| ProviderMessage::System {
        role: SystemRole::System,
        content: content.map(str::to_owned),
    };
    let [user, _, odd, _] = messages.as_slice() else {
        panic!("four messages")
    };
    let expected = vec![
        ProviderMessage::Unchanged(user.clone()),
        system(Some("be brief")),
        ProviderMessage::Unchanged(odd.clone()),
        system(None),
    ];
    assert_eq!(projected.unwrap(), expected);
    let notice =
        "DEBUG message Translating developer role to system role for non-OpenAI providers.";
    assert_eq!(logs, vec![notice.to_owned(), notice.to_owned()]);
    let missing_role = vec![record(&[("content", Some("hi"))])];
    assert_eq!(
        developer_to_system(missing_role),
        Err(ContextError::MissingRole { index: 0 })
    );
    let missing_content = vec![
        record(&[("role", Some("user"))]),
        record(&[("role", Some("developer"))]),
    ];
    let error = ContextError::MissingContent { index: 1 };
    assert_eq!(developer_to_system(missing_content), Err(error));
}

#[derive(Debug, PartialEq, Eq)]
struct FixtureStream {
    reads: usize,
}

fn iterator_initial_state() {
    let (state, logs) =
        capture(|| StreamState::<FixtureStream, ()>::from(FixtureStream { reads: 0 }));
    assert!(logs.is_empty());
    let expected = StreamState {
        stream: FixtureStream { reads: 0 },
        json_mode: Some(false),
        http_response: None,
    };
    assert_eq!(state, expected);
}

fn batch_initializer() {
    let (config, logs) = capture(|| BatchConfigBase);
    assert!(logs.is_empty());
    assert_eq!(size_of_val(&config), 0);
}

fn chat_id_prefix() {
    let id = Uuid::from_u128(0x0123_4567_89ab_4def_8123_4567_89ab_cdef);
    let rendered = String::from(ChatCompletionId(id));
    assert_eq!(rendered, "chatcmpl-01234567-89ab-4def-8123-456789abcdef");
}

struct Recorder;

impl ToolCallProjection<String> for Recorder {
    type Output = (&'static str, Record<String>);

    fn custom(&self, fields: Record<String>) -> Self::Output {
        ("custom", fields)
    }

    fn function(&self, fields: Record<String>) -> Self::Output {
        ("function", fields)
    }
}

fn tool_call_record_projection() {
    let typed = record(&[("type", Some("custom")), ("function", None), ("id", None)]);
    let expected = record(&[("type", Some("custom")), ("id", None)]);
    assert_eq!(project_tool_call(typed, &Recorder), ("custom", expected));
    let payload = record(&[
        ("custom", Some("{}")),
        ("type", None),
        ("function", Some("f")),
    ]);
    let kept = record(&[("custom", Some("{}")), ("function", Some("f"))]);
    assert_eq!(project_tool_call(payload, &Recorder), ("custom", kept));
    let function = record(&[("type", None), ("function", Some("f")), ("custom", None)]);
    assert_eq!(
        project_tool_call(function.clone(), &Recorder),
        ("function", function)
    );
}

#[rstest::rstest]
#[case::response_format_native(response_format_native)]
#[case::response_format_tool(response_format_tool)]
#[case::developer_role_projection(developer_role_projection)]
#[case::iterator_initial_state(iterator_initial_state)]
#[case::batch_initializer(batch_initializer)]
#[case::chat_id_prefix(chat_id_prefix)]
#[case::tool_call_record_projection(tool_call_record_projection)]
fn provider_message_primitives_cases(#[case] outcome: fn()) {
    outcome();
}
