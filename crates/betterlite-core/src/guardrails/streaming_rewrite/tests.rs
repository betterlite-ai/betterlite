#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use rstest::rstest;
use thiserror::Error;
use tracing::subscriber::set_default;

use super::capability::find_guardrail_callback;
use super::failure::UndeliverableStreamRewrite;
use super::legacy::LegacyHookStreamAdapter;
use super::observer::StreamRewriteObserver;
use super::shapes::{
    RequestData, StreamInputs, ToolShape, count_change, tool_call_mismatch_reason, tool_call_shape,
};
use super::step::{StepOutcome, StreamingStep, run_streaming_step};
use super::traits::{
    ApplySurface, CallbackEntry, GuardrailCall, GuardrailCallback, HookSurface, InputType,
    NamedGuardrail, PostCallHook, PostCallSurface, RewriteFailure, StreamCall, StreamGuardrail,
    StreamTranslation, UserApiKey,
};

#[derive(Debug, Clone, Error, PartialEq)]
enum FakeError {
    #[error(transparent)]
    Undeliverable(#[from] UndeliverableStreamRewrite),
    #[error("hook exploded")]
    Hook,
}

impl RewriteFailure for FakeError {
    fn undeliverable(&self) -> Option<&UndeliverableStreamRewrite> {
        match self {
            Self::Undeliverable(undeliverable) => Some(undeliverable),
            Self::Hook => None,
        }
    }
}

#[derive(Clone)]
enum Hook {
    Nothing,
    Replace(GuardrailValue),
    Mutate(GuardrailValue),
    Fail,
}

#[derive(Clone)]
struct Fake {
    name: Option<String>,
    texts_out: Option<Vec<String>>,
    tools_out: Option<Vec<GuardrailValue>>,
    apply_error: Option<FakeError>,
    surface: HookSurface,
    records_own: bool,
    covers: bool,
    hook: Hook,
}

const UNIFIED: HookSurface = HookSurface {
    apply: ApplySurface {
        own_apply_guardrail: true,
        use_native_lifecycle_hooks: false,
    },
    post_call: PostCallSurface {
        own_success_hook: false,
        own_streaming_iterator_hook: false,
    },
};

const LEGACY: HookSurface = HookSurface {
    apply: ApplySurface {
        own_apply_guardrail: false,
        use_native_lifecycle_hooks: false,
    },
    post_call: PostCallSurface {
        own_success_hook: true,
        own_streaming_iterator_hook: false,
    },
};

impl Fake {
    fn new(surface: HookSurface) -> Self {
        Self {
            name: Some("g".to_owned()),
            texts_out: None,
            tools_out: None,
            apply_error: None,
            surface,
            records_own: false,
            covers: false,
            hook: Hook::Nothing,
        }
    }
}

impl NamedGuardrail for Fake {
    fn guardrail_name(&self) -> Option<&str> {
        self.name.as_deref()
    }
}

impl StreamGuardrail<()> for Fake {
    type Failure = FakeError;

    fn covers_full_request(&self) -> bool {
        self.covers
    }

    fn apply_guardrail(
        &mut self,
        inputs: StreamInputs,
        _call: GuardrailCall<'_, ()>,
    ) -> impl Future<Output = Result<StreamInputs, FakeError>> + Send {
        if let Some(error) = self.apply_error.clone() {
            return ready(Err(error));
        }
        ready(Ok(StreamInputs {
            texts: self.texts_out.clone().or(inputs.texts),
            tool_calls: self.tools_out.clone().or(inputs.tool_calls),
            extra: inputs.extra,
        }))
    }
}

impl GuardrailCallback<()> for Fake {
    fn hook_surface(&self) -> HookSurface {
        self.surface
    }

    fn records_own_information(&self) -> bool {
        self.records_own
    }

    fn async_post_call_success_hook(
        &mut self,
        call: PostCallHook<'_>,
    ) -> impl Future<Output = Result<Option<GuardrailValue>, FakeError>> + Send {
        call.data.insert(
            "hook_user".to_owned(),
            GuardrailValue::Object(call.user_api_key_dict.clone()),
        );
        ready(match &self.hook {
            Hook::Nothing => Ok(None),
            Hook::Replace(value) => Ok(Some(value.clone())),
            Hook::Mutate(value) => {
                *call.response = Some(value.clone());
                Ok(None)
            }
            Hook::Fail => Err(FakeError::Hook),
        })
    }
}

#[derive(Clone, Copy)]
enum Stored {
    Assembled,
    Nothing,
    Null,
}

struct Translation<const DELIVERS: bool> {
    tools: Option<Vec<GuardrailValue>>,
    stored: Stored,
    rescans: bool,
}

impl<const DELIVERS: bool> Translation<DELIVERS> {
    fn new(tools: Option<Vec<GuardrailValue>>) -> Self {
        Self {
            tools,
            stored: Stored::Assembled,
            rescans: true,
        }
    }
}

fn texts_of(items: &[GuardrailValue]) -> Vec<String> {
    let mut texts = Vec::new();
    for item in items {
        if let GuardrailValue::Text(text) = item {
            texts.push(text.clone());
        }
    }
    texts
}

fn assembled(texts: &[&str], tools: &[GuardrailValue]) -> GuardrailValue {
    let text_items = texts
        .iter()
        .map(|text| GuardrailValue::Text((*text).to_owned()))
        .collect();
    GuardrailValue::Object(BTreeMap::from([
        ("texts".to_owned(), GuardrailValue::Array(text_items)),
        (
            "tool_calls".to_owned(),
            GuardrailValue::Array(tools.to_vec()),
        ),
    ]))
}

impl<const DELIVERS: bool> StreamTranslation<()> for Translation<DELIVERS> {
    type Failure = FakeError;

    const DELIVERS_ENDED_STREAM_REWRITES: bool = DELIVERS;

    async fn process_output_response<Guardrail>(
        &self,
        response: GuardrailValue,
        guardrail: &mut Guardrail,
        call: StreamCall<'_, ()>,
    ) -> Result<(), FakeError>
    where
        Guardrail: StreamGuardrail<(), Failure = FakeError>,
    {
        let GuardrailValue::Object(fields) = response else {
            return Ok(());
        };
        if !self.rescans {
            return Ok(());
        }
        let texts = match fields.get("texts") {
            Some(GuardrailValue::Array(items)) => Some(texts_of(items)),
            _ => None,
        };
        let tool_calls = match fields.get("tool_calls") {
            Some(GuardrailValue::Array(items)) => Some(items.clone()),
            _ => None,
        };
        let mut request_data = RequestData::new();
        let inputs = StreamInputs {
            texts,
            tool_calls,
            extra: BTreeMap::new(),
        };
        let call_args = GuardrailCall {
            request_data: &mut request_data,
            input_type: InputType::Response,
            logging_obj: call.logging_obj,
        };
        guardrail.apply_guardrail(inputs, call_args).await?;
        Ok(())
    }

    async fn process_streaming_response<Guardrail>(
        &self,
        responses_so_far: &mut Vec<GuardrailValue>,
        guardrail: &mut Guardrail,
        call: StreamCall<'_, ()>,
    ) -> Result<(), FakeError>
    where
        Guardrail: StreamGuardrail<(), Failure = FakeError>,
    {
        let request_data = call.request_data.unwrap();
        let texts = texts_of(responses_so_far);
        let text_refs: Vec<&str> = texts.iter().map(String::as_str).collect();
        let tools = self.tools.clone().unwrap_or_default();
        match self.stored {
            Stored::Assembled => {
                request_data.insert("response".to_owned(), assembled(&text_refs, &tools));
            }
            Stored::Null => {
                request_data.insert("response".to_owned(), GuardrailValue::Null);
            }
            Stored::Nothing => {}
        }
        let inputs = StreamInputs {
            texts: Some(texts),
            tool_calls: self.tools.clone(),
            extra: BTreeMap::new(),
        };
        let call_args = GuardrailCall {
            request_data,
            input_type: InputType::Response,
            logging_obj: call.logging_obj,
        };
        let outputs = guardrail.apply_guardrail(inputs, call_args).await?;
        if let Some(rewritten) = outputs.texts
            && rewritten.len() == responses_so_far.len()
        {
            *responses_so_far = rewritten.into_iter().map(GuardrailValue::Text).collect();
        }
        request_data.insert(
            "delivered_tools".to_owned(),
            GuardrailValue::Array(outputs.tool_calls.unwrap_or_default()),
        );
        Ok(())
    }
}

fn tool(name: &str, arguments: &str) -> GuardrailValue {
    let function = BTreeMap::from([
        ("name".to_owned(), GuardrailValue::Text(name.to_owned())),
        (
            "arguments".to_owned(),
            GuardrailValue::Text(arguments.to_owned()),
        ),
    ]);
    GuardrailValue::Object(BTreeMap::from([(
        "function".to_owned(),
        GuardrailValue::Object(function),
    )]))
}

fn chunks(texts: &[&str]) -> Vec<GuardrailValue> {
    texts
        .iter()
        .map(|text| GuardrailValue::Text((*text).to_owned()))
        .collect()
}

fn record_applied(request_data: &mut RequestData, guardrail_name: &str) {
    let entry = request_data
        .entry("applied".to_owned())
        .or_insert_with(|| GuardrailValue::Array(Vec::new()));
    if let GuardrailValue::Array(names) = entry {
        names.push(GuardrailValue::Text(guardrail_name.to_owned()));
    }
}

struct Run {
    outcome: Result<StepOutcome, FakeError>,
    chunks: Vec<GuardrailValue>,
    hook_input: RequestData,
}

async fn run<const DELIVERS: bool>(
    fake: &mut Fake,
    translation: &Translation<DELIVERS>,
    texts: &[&str],
) -> Run {
    let mut streaming_chunks = chunks(texts);
    let mut hook_input = RequestData::from([(
        "response".to_owned(),
        GuardrailValue::Text("stale".to_owned()),
    )]);
    let user = UserApiKey::from([("user".to_owned(), GuardrailValue::Text("u".to_owned()))]);
    let step = StreamingStep {
        guardrail: "g",
        streaming_chunks: &mut streaming_chunks,
        hook_input: &mut hook_input,
        user_api_key_dict: &user,
        logging_obj: None::<&mut ()>,
    };
    let outcome = run_streaming_step(fake, translation, step, record_applied).await;
    Run {
        outcome,
        chunks: streaming_chunks,
        hook_input,
    }
}

fn applied_names(hook_input: &RequestData) -> Option<&GuardrailValue> {
    hook_input.get("applied")
}

fn only_applied() -> GuardrailValue {
    GuardrailValue::Array(vec![GuardrailValue::Text("g".to_owned())])
}

const NO_WRITE_BACK: &str =
    "this endpoint's streaming pipeline does not write ended-stream rewrites back yet";
const COUNT_CHANGE: &str = "the guardrail returned 2 tool calls for a stream that carried 1, and a rewrite that drops or adds a tool call cannot be written back";

#[derive(Clone, Copy)]
enum Expect {
    Applied { rewritten: bool },
    Discarded(&'static str),
}

#[rstest]
#[case::text_rewrite(Some(&["A", "B"][..]), None, false, Expect::Applied { rewritten: true })]
#[case::no_rewrite(None, None, false, Expect::Applied { rewritten: false })]
#[case::same_text_is_no_rewrite(Some(&["a", "b"][..]), None, false, Expect::Applied { rewritten: false })]
#[case::tool_rewrite(None, Some(vec![tool("f", "{\"x\":2}")]), false, Expect::Applied { rewritten: false })]
#[case::tool_added(None, Some(vec![tool("f", "{}"), tool("g", "{}")]), false, Expect::Discarded(COUNT_CHANGE))]
#[case::count_change_discards_text_too(Some(&["A", "B"][..]), Some(vec![tool("f", "{}"), tool("g", "{}")]), false, Expect::Discarded(COUNT_CHANGE))]
#[case::own_information(Some(&["A", "B"][..]), None, true, Expect::Applied { rewritten: true })]
#[tokio::test]
async fn streaming_rewrite_delivers(
    #[case] texts_out: Option<&[&str]>,
    #[case] tools_out: Option<Vec<GuardrailValue>>,
    #[case] records_own: bool,
    #[case] expect: Expect,
) {
    let mut fake = Fake::new(UNIFIED);
    fake.texts_out = texts_out.map(|items| items.iter().map(|item| (*item).to_owned()).collect());
    fake.tools_out.clone_from(&tools_out);
    fake.records_own = records_own;
    let translation = Translation::<true>::new(Some(vec![tool("f", "{}")]));
    let result = run(&mut fake, &translation, &["a", "b"]).await;
    match expect {
        Expect::Applied { rewritten } => {
            assert_eq!(result.outcome, Ok(StepOutcome::Applied));
            let expected_chunks = if rewritten {
                chunks(&["A", "B"])
            } else {
                chunks(&["a", "b"])
            };
            assert_eq!(result.chunks, expected_chunks);
            let expected_names = only_applied();
            let expected = (!records_own).then_some(&expected_names);
            assert_eq!(applied_names(&result.hook_input), expected);
            let delivered = tools_out.unwrap_or_else(|| vec![tool("f", "{}")]);
            assert_eq!(
                result.hook_input.get("delivered_tools"),
                Some(&GuardrailValue::Array(delivered))
            );
        }
        Expect::Discarded(reason) => {
            let outcome = StepOutcome::Discarded {
                reason: reason.to_owned(),
            };
            assert_eq!(result.outcome, Ok(outcome));
            assert_eq!(result.chunks, chunks(&["a", "b"]));
            assert_eq!(applied_names(&result.hook_input), None);
        }
    }
}

#[rstest]
#[case::text_rewrite(Some(&["A", "B"][..]), None, Expect::Discarded(NO_WRITE_BACK))]
#[case::tool_rewrite(None, Some(vec![tool("f", "{\"x\":2}")]), Expect::Discarded(NO_WRITE_BACK))]
#[case::no_rewrite(None, None, Expect::Applied { rewritten: false })]
#[case::count_change_first(Some(&["A", "B"][..]), Some(vec![tool("f", "{}"), tool("g", "{}")]), Expect::Discarded(COUNT_CHANGE))]
#[tokio::test]
async fn streaming_rewrite_no_write_back(
    #[case] texts_out: Option<&[&str]>,
    #[case] tools_out: Option<Vec<GuardrailValue>>,
    #[case] expect: Expect,
) {
    let mut fake = Fake::new(UNIFIED);
    fake.texts_out = texts_out.map(|items| items.iter().map(|item| (*item).to_owned()).collect());
    fake.tools_out = tools_out;
    let translation = Translation::<false>::new(Some(vec![tool("f", "{}")]));
    let result = run(&mut fake, &translation, &["a", "b"]).await;
    assert_eq!(result.chunks, chunks(&["a", "b"]));
    match expect {
        Expect::Applied { .. } => {
            assert_eq!(result.outcome, Ok(StepOutcome::Applied));
            assert_eq!(applied_names(&result.hook_input), Some(&only_applied()));
        }
        Expect::Discarded(reason) => {
            let outcome = StepOutcome::Discarded {
                reason: reason.to_owned(),
            };
            assert_eq!(result.outcome, Ok(outcome));
            assert_eq!(applied_names(&result.hook_input), None);
        }
    }
}

#[rstest]
#[case::undeliverable_error(
    FakeError::Undeliverable(UndeliverableStreamRewrite { guardrail_name: "g".to_owned(), reason: "nope".to_owned() }),
    Ok(StepOutcome::Discarded { reason: "nope".to_owned() })
)]
#[case::other_error(FakeError::Hook, Err(FakeError::Hook))]
#[tokio::test]
async fn streaming_rewrite_errors(
    #[case] error: FakeError,
    #[case] expected: Result<StepOutcome, FakeError>,
) {
    let mut fake = Fake::new(UNIFIED);
    fake.apply_error = Some(error);
    let translation = Translation::<true>::new(None);
    let result = run(&mut fake, &translation, &["a"]).await;
    assert_eq!(result.outcome, expected);
    assert_eq!(result.chunks, chunks(&["a"]));
    assert_eq!(applied_names(&result.hook_input), None);
}

#[tokio::test]
async fn streaming_rewrite_drops_stale_response() {
    let mut fake = Fake::new(UNIFIED);
    let mut translation = Translation::<true>::new(None);
    translation.stored = Stored::Nothing;
    let result = run(&mut fake, &translation, &["a"]).await;
    assert_eq!(result.hook_input.get("response"), None);
}

#[rstest]
#[case::replace_texts(Hook::Replace(assembled(&["A", "B"], &[tool("f", "{}")])), Stored::Assembled, true, Expect::Applied { rewritten: true })]
#[case::mutate_in_place(Hook::Mutate(assembled(&["A", "B"], &[tool("f", "{}")])), Stored::Assembled, true, Expect::Applied { rewritten: true })]
#[case::untouched(Hook::Nothing, Stored::Assembled, true, Expect::Applied { rewritten: false })]
#[case::no_stored_response(Hook::Nothing, Stored::Nothing, true, Expect::Applied { rewritten: false })]
#[case::null_response_is_absent(Hook::Nothing, Stored::Null, true, Expect::Applied { rewritten: false })]
#[case::fewer_texts(Hook::Replace(assembled(&["A"], &[tool("f", "{}")])), Stored::Assembled, true, Expect::Discarded("the legacy hook returned 1 texts for a stream that carried 2"))]
#[case::tool_changed(Hook::Replace(assembled(&["a", "b"], &[tool("f", "{\"x\":2}")])), Stored::Assembled, true, Expect::Discarded("the legacy hook changed a tool call's name or arguments, which this path cannot write back"))]
#[case::tool_dropped(Hook::Replace(assembled(&["a", "b"], &[])), Stored::Assembled, true, Expect::Discarded("the legacy hook returned 0 tool calls for a stream that carried 1"))]
#[case::not_rescannable(
    Hook::Nothing,
    Stored::Assembled,
    false,
    Expect::Discarded(
        "the legacy hook's response could not be rescanned by this endpoint's translation"
    )
)]
#[tokio::test]
async fn streaming_rewrite_legacy_hook(
    #[case] hook: Hook,
    #[case] stored: Stored,
    #[case] rescans: bool,
    #[case] expect: Expect,
) {
    let mut fake = Fake::new(LEGACY);
    fake.hook = hook;
    let mut translation = Translation::<true>::new(Some(vec![tool("f", "{}")]));
    translation.stored = stored;
    translation.rescans = rescans;
    let result = run(&mut fake, &translation, &["a", "b"]).await;
    let user = GuardrailValue::Object(BTreeMap::from([(
        "user".to_owned(),
        GuardrailValue::Text("u".to_owned()),
    )]));
    assert_eq!(result.hook_input.get("hook_user"), Some(&user));
    match expect {
        Expect::Applied { rewritten } => {
            assert_eq!(result.outcome, Ok(StepOutcome::Applied));
            let expected = if rewritten {
                chunks(&["A", "B"])
            } else {
                chunks(&["a", "b"])
            };
            assert_eq!(result.chunks, expected);
            assert_eq!(applied_names(&result.hook_input), Some(&only_applied()));
        }
        Expect::Discarded(reason) => {
            let outcome = StepOutcome::Discarded {
                reason: reason.to_owned(),
            };
            assert_eq!(result.outcome, Ok(outcome));
            assert_eq!(result.chunks, chunks(&["a", "b"]));
            assert_eq!(applied_names(&result.hook_input), None);
        }
    }
}

#[tokio::test]
async fn streaming_rewrite_legacy_tool_only() {
    let mut fake = Fake::new(LEGACY);
    let translation = Translation::<true>::new(Some(vec![tool("f", "{}")]));
    let result = run(&mut fake, &translation, &[]).await;
    assert_eq!(result.outcome, Ok(StepOutcome::Applied));
    assert_eq!(applied_names(&result.hook_input), Some(&only_applied()));
}

#[tokio::test]
async fn streaming_rewrite_legacy_hook_error() {
    let mut fake = Fake::new(LEGACY);
    fake.hook = Hook::Fail;
    let translation = Translation::<true>::new(None);
    let result = run(&mut fake, &translation, &["a"]).await;
    assert_eq!(result.outcome, Err(FakeError::Hook));
    assert_eq!(result.chunks, chunks(&["a"]));
}

#[rstest]
#[case::named(Some("g"), "g")]
#[case::unnamed(None, "unknown")]
#[tokio::test]
async fn streaming_rewrite_adapter_error(#[case] name: Option<&str>, #[case] expected: &str) {
    let mut fake = Fake::new(LEGACY);
    fake.name = name.map(str::to_owned);
    let mut rescan_less = Translation::<true>::new(None);
    rescan_less.rescans = false;
    let user = UserApiKey::new();
    let mut adapter = LegacyHookStreamAdapter {
        inner: &mut fake,
        endpoint_translation: &rescan_less,
        user_api_key_dict: &user,
    };
    let mut request_data = RequestData::from([("response".to_owned(), assembled(&["a"], &[]))]);
    let call = GuardrailCall {
        request_data: &mut request_data,
        input_type: InputType::Response,
        logging_obj: None::<&mut ()>,
    };
    let inputs = StreamInputs {
        texts: Some(vec!["a".to_owned()]),
        tool_calls: None,
        extra: BTreeMap::new(),
    };
    let result = adapter.apply_guardrail(inputs, call).await;
    let Err(FakeError::Undeliverable(undeliverable)) = result else {
        panic!("expected an undeliverable rewrite");
    };
    assert_eq!(undeliverable.guardrail_name, expected);
}

#[tokio::test]
async fn streaming_rewrite_observer_sticks() {
    let mut fake = Fake::new(UNIFIED);
    fake.covers = true;
    let mut observer = StreamRewriteObserver::new(&mut fake);
    assert_eq!(observer.guardrail_name(), Some("g"));
    assert!(StreamGuardrail::<()>::covers_full_request(&observer));
    let mut request_data = RequestData::new();
    let sent = StreamInputs {
        texts: Some(vec!["a".to_owned()]),
        tool_calls: Some(vec![tool("f", "{}")]),
        extra: BTreeMap::new(),
    };
    observer.inner.texts_out = Some(vec!["b".to_owned()]);
    observer.inner.tools_out = Some(vec![tool("f", "{}"), tool("f", "{}")]);
    let call = GuardrailCall {
        request_data: &mut request_data,
        input_type: InputType::Response,
        logging_obj: None::<&mut ()>,
    };
    observer.apply_guardrail(sent.clone(), call).await.unwrap();
    observer.inner.texts_out = None;
    observer.inner.tools_out = Some(vec![tool("f", "{}"); 3]);
    let second_call = GuardrailCall {
        request_data: &mut request_data,
        input_type: InputType::Response,
        logging_obj: None::<&mut ()>,
    };
    observer.apply_guardrail(sent, second_call).await.unwrap();
    assert!(observer.rewrote_texts);
    assert!(observer.rewrote_tool_calls);
    assert_eq!(observer.tool_call_count_change, Some((1, 2)));
}

#[test]
fn streaming_rewrite_undeliverable_message() {
    let undeliverable = UndeliverableStreamRewrite {
        guardrail_name: "pii".to_owned(),
        reason: "because".to_owned(),
    };
    assert_eq!(
        undeliverable.to_string(),
        "Guardrail 'pii' rewrote the streamed response but the rewrite cannot be written back to the stream: because"
    );
}

#[derive(Clone, Copy)]
enum Entry {
    Guardrail(&'static str),
    Unnamed,
    Other,
}

struct Named(Option<&'static str>);

impl NamedGuardrail for Named {
    fn guardrail_name(&self) -> Option<&str> {
        self.0
    }
}

struct Registered(Entry, Named);

impl CallbackEntry for Registered {
    type Guardrail = Named;

    fn as_guardrail(&self) -> Option<&Named> {
        match self.0 {
            Entry::Other => None,
            Entry::Guardrail(_) | Entry::Unnamed => Some(&self.1),
        }
    }
}

fn registered(entry: Entry) -> Registered {
    let name = match entry {
        Entry::Guardrail(name) => Some(name),
        Entry::Unnamed | Entry::Other => None,
    };
    Registered(entry, Named(name))
}

#[rstest]
#[case::found(&[Entry::Other, Entry::Guardrail("a"), Entry::Guardrail("b")], "b", true)]
#[case::missing(&[Entry::Guardrail("a")], "b", false)]
#[case::unnamed_never_matches(&[Entry::Unnamed], "", false)]
#[case::empty(&[], "a", false)]
fn streaming_rewrite_find_callback(
    #[case] entries: &[Entry],
    #[case] wanted: &str,
    #[case] found: bool,
) {
    let callbacks: Vec<Registered> = entries.iter().map(|entry| registered(*entry)).collect();
    let result = find_guardrail_callback(wanted, &callbacks);
    assert_eq!(result.is_some(), found);
    assert_eq!(
        result.and_then(NamedGuardrail::guardrail_name),
        found.then_some(wanted)
    );
}

#[rstest]
#[case::unified([true, false, false, false], [true, true])]
#[case::native_hooks_only([true, true, false, false], [false, false])]
#[case::native_with_post_call([true, true, true, false], [false, true])]
#[case::post_call_only([false, false, true, false], [false, true])]
#[case::post_call_and_iterator([false, false, true, true], [false, false])]
#[case::iterator_only([false, false, false, true], [false, false])]
#[case::neither([false, false, false, false], [false, false])]
fn streaming_rewrite_capabilities(#[case] flags: [bool; 4], #[case] expected: [bool; 2]) {
    let [own_apply, native, success, iterator] = flags;
    let fake = Fake::new(HookSurface {
        apply: ApplySurface {
            own_apply_guardrail: own_apply,
            use_native_lifecycle_hooks: native,
        },
        post_call: PostCallSurface {
            own_success_hook: success,
            own_streaming_iterator_hook: iterator,
        },
    });
    let actual = [
        GuardrailCallback::<()>::supports_unified_execution(&fake),
        GuardrailCallback::<()>::supports_streaming_execution(&fake),
    ];
    assert_eq!(actual, expected);
}

#[rstest]
#[case::not_an_object(GuardrailValue::Text("x".to_owned()), ToolShape::default())]
#[case::no_function(GuardrailValue::Object(BTreeMap::new()), ToolShape::default())]
#[case::function_not_object(
    GuardrailValue::Object(BTreeMap::from([("function".to_owned(), GuardrailValue::Null)])),
    ToolShape::default()
)]
#[case::complete(
    tool("f", "{}"),
    ToolShape { name: Some(GuardrailValue::Text("f".to_owned())), arguments: Some(GuardrailValue::Text("{}".to_owned())) }
)]
#[case::null_name_is_absent(
    GuardrailValue::Object(BTreeMap::from([("function".to_owned(), GuardrailValue::Object(BTreeMap::from([
        ("name".to_owned(), GuardrailValue::Null),
        ("arguments".to_owned(), GuardrailValue::Null),
    ])))])),
    ToolShape::default()
)]
fn streaming_rewrite_tool_shape(#[case] tool_call: GuardrailValue, #[case] expected: ToolShape) {
    assert_eq!(tool_call_shape(&tool_call), expected);
}

#[rstest]
#[case::equal(Some(&["a"][..]), Some(&["a"][..]), None)]
#[case::none_equals_none(None, None, None)]
#[case::none_differs_from_empty(None, Some(&[][..]), Some((0, 0)))]
#[case::same_count(Some(&["a"][..]), Some(&["b"][..]), Some((1, 1)))]
#[case::different_count(Some(&["a"][..]), Some(&["a", "b"][..]), Some((1, 2)))]
fn streaming_rewrite_mismatch(
    #[case] sent: Option<&[&str]>,
    #[case] returned: Option<&[&str]>,
    #[case] counts: Option<(usize, usize)>,
) {
    let shape = |value: &&str| ToolShape {
        name: Some(GuardrailValue::Text((*value).to_owned())),
        arguments: None,
    };
    let sent_shapes: Option<Vec<ToolShape>> = sent.map(|items| items.iter().map(shape).collect());
    let returned_shapes: Option<Vec<ToolShape>> =
        returned.map(|items| items.iter().map(shape).collect());
    let reason = tool_call_mismatch_reason(sent_shapes.as_deref(), returned_shapes.as_deref());
    let expected = counts.map(|(sent_count, returned_count)| {
        if sent_count == returned_count {
            "the legacy hook changed a tool call's name or arguments, which this path cannot write back".to_owned()
        } else {
            format!("the legacy hook returned {returned_count} tool calls for a stream that carried {sent_count}")
        }
    });
    assert_eq!(reason, expected);
    let changed = count_change(sent_shapes.as_deref(), returned_shapes.as_deref());
    let expected_change =
        counts.filter(|(sent_count, returned_count)| sent_count != returned_count);
    assert_eq!(changed, expected_change);
}

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn streaming_rewrite_logs_discard() {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_ansi(false)
        .finish();
    let guard = set_default(subscriber);
    let mut fake = Fake::new(UNIFIED);
    fake.texts_out = Some(vec!["A".to_owned()]);
    let translation = Translation::<false>::new(None);
    run(&mut fake, &translation, &["a"]).await;
    drop(guard);
    let logged = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
    assert!(logged.contains("WARN"), "{logged}");
    assert!(logged.contains("guardrail=\"g\""), "{logged}");
    assert!(logged.contains(NO_WRITE_BACK), "{logged}");
}
