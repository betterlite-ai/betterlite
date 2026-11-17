use betterlite_http::error::HttpError;
use betterlite_models::decisions::Json;
use betterlite_streaming::stream_chunk_shaping::records::{GenericChunk, JsonValue};
use rstest::rstest;

use super::support::{
    BASE, Call, Consume, DATA, FIRST, Inputs, Outcome, REPLY, Refusal, Reply, converse, invoke,
    member, messages, run, run_default, run_supplied, sorted_pairs,
};
use crate::bedrock_http_stream::error::HttpStreamError;
use crate::bedrock_http_stream::stream::StreamFailure;
use crate::bedrock_settings::ResponseBody;
use crate::converse_events::ChunkProjection;
use crate::event_frames::error::EventFrameError;
use crate::invoke_events::InvokeChunk;
use crate::invoke_events::error::InvokeStreamError;

const START: &str = "0000008e0000002ab04f184c0b3a6576656e742d747970650700056368756e6b0d3a6d6573736167652d747970650700056576656e747b226279746573223a2265794a7a644746796443493665794a796232786c496a6f6959584e7a61584e3059573530496e3073496d4e76626e526c626e52436247396a61306c755a475634496a6f7766513d3d227df9b9a0e5";
const HELLO: &str = "000000860000002a803f538d0b3a6576656e742d747970650700056368756e6b0d3a6d6573736167652d747970650700056576656e747b226279746573223a2265794a6b5a5778305953493665794a305a586830496a6f69614756736247386966537769593239756447567564454a7362324e725357356b5a5867694f6a4239227d23df9631";
const WORLD: &str = "0000008a0000002a45cfbe8c0b3a6576656e742d747970650700056368756e6b0d3a6d6573736167652d747970650700056576656e747b226279746573223a2265794a6b5a5778305953493665794a305a586830496a6f6949486476636d786b496e3073496d4e76626e526c626e52436247396a61306c755a475634496a6f7766513d3d227d3fd85d87";
const STOP: &str = "0000006a0000002a6d1b79530b3a6576656e742d747970650700056368756e6b0d3a6d6573736167652d747970650700056576656e747b226279746573223a2265794a7a64473977556d566863323975496a6f695a57356b58335231636d346966513d3d227d7210a35f";
const USAGE: &str = "000000be0000002a116ea0ca0b3a6576656e742d747970650700056368756e6b0d3a6d6573736167652d747970650700056576656e747b226279746573223a2265794a745a58527961574e7a496a7037496d78686447567559336c4e637949364f583073496e567a5957646c496a7037496d6c7563485630564739725a57357a496a6f784d4377696233563063485630564739725a57357a496a6f314c434a30623352686246527661325675637949364d54563966513d3d227da255de78";
// Sixteen zero bytes: enough for a prelude, whose checksum cannot match.
const MALFORMED: &str = "00000000000000000000000000000000";

const TOOL_REPLY: &str = r#"{"output":{"message":{"role":"assistant","content":[{"toolUse":{"toolUseId":"tool-1","name":"top_song","input":{"sign":"WZPZ"}}}]}},"stopReason":"tool_use","usage":{"inputTokens":2,"outputTokens":3,"totalTokens":5}}"#;

// A buffered reply whose only tool is the synthetic response-format call a json_mode client gets.
const JSON_TOOL_REPLY: &str = r#"{"output":{"message":{"role":"assistant","content":[{"toolUse":{"toolUseId":"tool-1","name":"json_tool_call","input":{"sign":"WZPZ"}}}]}},"stopReason":"tool_use","usage":{"inputTokens":2,"outputTokens":3,"totalTokens":5}}"#;

// One DeepSeek R1 chunk event: {"generation":"hello"}.
const DEEPSEEK_CHUNK: &str = "000000660000002aa8eb94520b3a6576656e742d747970650700056368756e6b0d3a6d6573736167652d747970650700056576656e747b226279746573223a2265794a6e5a57356c636d463061573975496a6f69614756736247386966513d3d227da8f89c66";

// A Converse content-block start, tool delta and stop, each carrying a json_tool_call block.
const JSON_TOOL_START: &str = "000000d60000002ab82c9c460b3a6576656e742d747970650700056368756e6b0d3a6d6573736167652d747970650700056576656e747b226279746573223a2265794a6a623235305a573530516d787659327454644746796443493665794a7a644746796443493665794a306232397356584e6c496a7037496e5276623278566332564a5a434936496e517849697769626d46745a534936496d707a62323566644739766246396a59577873496e31394c434a6a623235305a573530516d78765932744a626d526c654349364d483139227d400de46f";
const JSON_TOOL_DELTA: &str = "000000ca0000002a1d3ce6c50b3a6576656e742d747970650700056368756e6b0d3a6d6573736167652d747970650700056576656e747b226279746573223a2265794a6a623235305a573530516d7876593274455a5778305953493665794a6b5a5778305953493665794a306232397356584e6c496a7037496d6c7563485630496a6f696531776963326c6e626c77694f694263496c646155467063496e306966583073496d4e76626e526c626e52436247396a61306c755a475634496a6f776658303d227d56ee3eb6";
const JSON_TOOL_STOP: &str = "000000820000002a75bff54d0b3a6576656e742d747970650700056368756e6b0d3a6d6573736167652d747970650700056576656e747b226279746573223a2265794a6a623235305a573530516d78765932745464473977496a7037496d4e76626e526c626e52436247396a61306c755a475634496a6f776658303d227d2340327a";

fn tool_member<'value>(tool: &'value JsonValue, name: &str) -> &'value JsonValue {
    match tool {
        JsonValue::Object(members) => members.get(name).expect("a present member"),
        JsonValue::Null
        | JsonValue::Bool(_)
        | JsonValue::Number(_)
        | JsonValue::String(_)
        | JsonValue::Array(_) => panic!("a tool call is an object"),
    }
}

fn generic(chunk: &InvokeChunk) -> &GenericChunk {
    match chunk {
        InvokeChunk::Base(projection) => match projection.as_ref() {
            ChunkProjection::Generic(generic) => generic,
            ChunkProjection::Converse(_) => panic!("the fake stream yields a generic chunk"),
        },
        InvokeChunk::Anthropic(_) | InvokeChunk::DeepSeek(_) | InvokeChunk::OpenAiCompatible(_) => {
            panic!("the fake stream is built on the base projection")
        }
    }
}

enum Field {
    Model,
    Content,
    FinishReason,
}

fn chunk_field(chunk: &InvokeChunk, field: Field) -> Option<&str> {
    match chunk {
        InvokeChunk::Base(projection) => match projection.as_ref() {
            ChunkProjection::Converse(converse) => match field {
                Field::Model => Some(converse.model.as_str()),
                Field::Content => converse.delta.content.as_deref(),
                Field::FinishReason => converse.finish_reason.as_deref(),
            },
            ChunkProjection::Generic(generic) => match field {
                Field::Model => None,
                Field::Content => Some(generic.text.as_str()),
                Field::FinishReason => Some(generic.finish_reason.as_str()),
            },
        },
        InvokeChunk::Anthropic(_) | InvokeChunk::DeepSeek(_) | InvokeChunk::OpenAiCompatible(_) => {
            None
        }
    }
}

// The message the tally built for a 502, which names the response headers the stream carried.
fn failure_message(failure: &StreamFailure) -> Option<&str> {
    match failure {
        StreamFailure::Decode(InvokeStreamError::Frame(EventFrameError::Bedrock {
            error, ..
        })) => Some(error.error.message.as_str()),
        StreamFailure::Read(_) | StreamFailure::Decode(_) => None,
    }
}

// A status other than 200 fails before any frame is read, with the reason phrase as its message.
fn assert_status_failure(status: u16, reason: &str, messages: &Json) {
    let mut reply = Reply::frames(&[]);
    reply.status = status;
    let inputs = Inputs::new(reply);
    let outcome = run(
        Call::Invoke(invoke(BASE, DATA, messages)),
        &inputs,
        Consume::None,
    );
    assert_eq!(outcome.provider_status(), Some(status));
    assert_eq!(outcome.provider_message(), Some(reason));
}

fn content_of(chunks: &[InvokeChunk]) -> Vec<&str> {
    chunks
        .iter()
        .filter_map(|chunk| chunk_field(chunk, Field::Content))
        .collect()
}

#[rstest]
#[case::case_1_invoke_stream_url(case_1_invoke_stream_url)]
#[case::case_2_http_error_before_frame(case_2_http_error_before_frame)]
#[case::case_3_converse_stream_headers(case_3_converse_stream_headers)]
#[case::case_4_fake_stream_projection(case_4_fake_stream_projection)]
#[case::case_5_post_call_before_consumption(case_5_post_call_before_consumption)]
fn bedrock_http_stream_cases(#[case] case: fn()) {
    case();
}

fn case_1_invoke_stream_url() {
    let messages = messages();
    invoke_posts(&messages);
    supplied_client_wins(&messages);
    invoke_signed_bodies(&messages);
    fake_streams_do_not_stream(&messages);
}

fn invoke_posts(messages: &Json) {
    let api_base = "https://bedrock-runtime.us-east-1.amazonaws.com/custom/invoke?trace=1";
    let inputs = Inputs::new(Reply::frames(&[FIRST]));
    let outcome = run(
        Call::Invoke(invoke(api_base, DATA, messages)),
        &inputs,
        Consume::None,
    );
    assert!(outcome.error.is_none());
    let posts = inputs.posts();
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].url, api_base);
    assert_eq!(posts[0].body, DATA);
    assert!(posts[0].stream);
    assert_eq!(
        posts[0].headers,
        vec![("content-type".to_owned(), "application/json".to_owned())]
    );
    assert_eq!(inputs.calls(), 1);

    // With no supplied client the helper posts through the default handler it was handed.
    let default_inputs = Inputs::new(Reply::frames(&[FIRST]));
    let default_outcome = run_default(
        Call::Invoke(invoke(api_base, DATA, messages)),
        &default_inputs,
        Consume::None,
    );
    assert!(default_outcome.error.is_none());
    assert_eq!(default_inputs.calls(), 1);
    assert_eq!(default_inputs.posts()[0].url, api_base);
}

fn supplied_client_wins(messages: &Json) {
    // A supplied client wins over the default handler, which then sees no request.
    let api_base = "https://bedrock-runtime.us-east-1.amazonaws.com/custom/invoke?trace=1";
    let supplied_inputs = Inputs::new(Reply::frames(&[FIRST]));
    let default = Inputs::new(Reply::refusing(Refusal::Closed));
    let supplied_outcome = run_supplied(
        Call::Invoke(invoke(api_base, DATA, messages)),
        &supplied_inputs,
        &default,
        Consume::None,
    );
    assert!(supplied_outcome.error.is_none());
    assert_eq!(supplied_inputs.calls(), 1);
    assert_eq!(default.calls(), 0);
}

fn invoke_signed_bodies(messages: &Json) {
    async_ignores_signed_body(messages);
    sync_sends_signed_body(messages);
}

fn async_ignores_signed_body(messages: &Json) {
    let api_base = "https://bedrock-runtime.us-east-1.amazonaws.com/custom/invoke?trace=1";

    // The async helper never reads a signed body, even one a caller supplied.
    let async_inputs = Inputs::new(Reply::frames(&[FIRST]));
    let mut async_call = invoke(api_base, DATA, messages);
    async_call.signed_json_body = Some(b"signed-async");
    let async_outcome = run(Call::Invoke(async_call), &async_inputs, Consume::None);
    assert!(async_outcome.error.is_none());
    assert_eq!(async_inputs.posts()[0].body, DATA);
}

fn sync_sends_signed_body(messages: &Json) {
    let api_base = "https://bedrock-runtime.us-east-1.amazonaws.com/custom/invoke?trace=1";

    // The sync helper sends the signed body whenever one exists, empty bytes included.
    let signed_inputs = Inputs::new(Reply::frames(&[FIRST]));
    let mut signed_call = invoke(api_base, DATA, messages);
    signed_call.sync = true;
    signed_call.signed_json_body = Some(b"signed-json");
    let signed_outcome = run(Call::Invoke(signed_call), &signed_inputs, Consume::None);
    assert!(signed_outcome.error.is_none());
    assert_eq!(signed_inputs.posts()[0].body, "signed-json");
    // The post-call log keeps the request data the caller passed, not the signed bytes on the wire.
    assert_eq!(
        member(
            member(signed_outcome.logged(0), "additional_args"),
            "complete_input_dict"
        ),
        &Json::Text(DATA.to_owned())
    );

    let empty_signed_inputs = Inputs::new(Reply::frames(&[FIRST]));
    let mut empty_signed_call = invoke(api_base, DATA, messages);
    empty_signed_call.sync = true;
    empty_signed_call.signed_json_body = Some(b"");
    let empty_signed_outcome = run(
        Call::Invoke(empty_signed_call),
        &empty_signed_inputs,
        Consume::None,
    );
    assert!(empty_signed_outcome.error.is_none());
    assert_eq!(empty_signed_inputs.posts()[0].body, "");

    let plain_inputs = Inputs::new(Reply::frames(&[FIRST]));
    let mut plain_call = invoke(api_base, DATA, messages);
    plain_call.sync = true;
    let plain_outcome = run(Call::Invoke(plain_call), &plain_inputs, Consume::None);
    assert!(plain_outcome.error.is_none());
    assert_eq!(plain_inputs.posts()[0].body, DATA);
}

fn fake_streams_do_not_stream(messages: &Json) {
    let api_base = "https://bedrock-runtime.us-east-1.amazonaws.com/custom/invoke?trace=1";

    // A fake stream tells the client not to stream, on every helper.
    let fake_invoke_inputs = Inputs::new(Reply::buffered(REPLY));
    let mut fake_invoke_call = invoke(api_base, DATA, messages);
    fake_invoke_call.fake_stream = true;
    let fake_invoke_outcome = run(
        Call::Invoke(fake_invoke_call),
        &fake_invoke_inputs,
        Consume::None,
    );
    assert!(fake_invoke_outcome.error.is_none());
    assert!(!fake_invoke_inputs.posts()[0].stream);

    let fake_converse_inputs = Inputs::new(Reply::buffered(REPLY));
    let mut fake_converse_call = converse(messages);
    fake_converse_call.api_base = api_base;
    fake_converse_call.fake_stream = true;
    let fake_converse_outcome = run(
        Call::Converse(fake_converse_call),
        &fake_converse_inputs,
        Consume::None,
    );
    assert!(fake_converse_outcome.error.is_none());
    assert!(!fake_converse_inputs.posts()[0].stream);
}

fn case_2_http_error_before_frame() {
    let messages = messages();
    invoke_status_failure(&messages);
    fake_body_failures(&messages);
    fake_read_failures(&messages);
    transport_refusals(&messages);
    converse_status_failure(&messages);
    converse_bytes_repr(&messages);
    converse_passthrough_failures(&messages);
}

fn invoke_status_failure(messages: &Json) {
    let mut reply = Reply::frames(&["69676e6f726564"]);
    reply.status = 400;
    let inputs = Inputs::new(reply);
    let outcome = run(
        Call::Invoke(invoke(BASE, DATA, messages)),
        &inputs,
        Consume::None,
    );
    assert_eq!(outcome.provider_status(), Some(400));
    assert_eq!(outcome.provider_message(), Some("Bad Request"));
    assert!(matches!(
        outcome.provider_body(),
        Some(ResponseBody::Unread { reason_phrase }) if reason_phrase == "Bad Request"
    ));
    assert_eq!(
        outcome.provider().unwrap().response_headers,
        vec![(
            "content-type".to_owned(),
            "application/vnd.amazon.eventstream".to_owned()
        )]
    );
    assert_eq!(inputs.polls(), 0);
    assert!(outcome.logs.is_empty());

    // Any other status is a failure too, named by its reason phrase.
    assert_status_failure(201, "Created", messages);
    assert_status_failure(204, "No Content", messages);
}

fn fake_body_failures(messages: &Json) {
    // A fake stream has read the body, so the failure carries its text.
    let mut fake_reply = Reply::buffered("boom");
    fake_reply.status = 500;
    let fake_inputs = Inputs::new(fake_reply);
    let mut fake_call = invoke(BASE, DATA, messages);
    fake_call.fake_stream = true;
    let fake_outcome = run(Call::Invoke(fake_call), &fake_inputs, Consume::None);
    assert_eq!(fake_outcome.provider_status(), Some(500));
    assert_eq!(fake_outcome.provider_message(), Some("boom"));
    assert!(
        matches!(fake_outcome.provider_body(), Some(ResponseBody::Read(text)) if text == "boom")
    );

    // A buffered reply that is not text is named by its byte count, the choice B607 records.
    let binary_inputs = Inputs::new(Reply {
        status: 500,
        headers: Vec::new(),
        reads: vec![Ok(b"\xff\xfe".to_vec())],
        refused: None,
    });
    let mut binary_call = invoke(BASE, DATA, messages);
    binary_call.fake_stream = true;
    let binary_outcome = run(Call::Invoke(binary_call), &binary_inputs, Consume::None);
    assert_eq!(binary_outcome.provider_status(), Some(500));
    assert_eq!(
        binary_outcome.provider_message(),
        Some("2 bytes that are not text")
    );
}

fn fake_read_failures(messages: &Json) {
    let timeout_outcome =
        fake_invoke_failure(messages, Reply::reading(500, Err(HttpError::Timeout)));
    assert_eq!(timeout_outcome.provider_status(), Some(408));
    assert_eq!(
        timeout_outcome.provider_message(),
        Some("Timeout error occurred.")
    );

    // A read failure that is not a timeout is the catch-all's 500.
    let too_large = Reply::reading(500, Err(HttpError::TooLarge { limit: 7 }));
    let failed_read_outcome = fake_invoke_failure(messages, too_large);
    assert_eq!(failed_read_outcome.provider_status(), Some(500));
    assert!(
        failed_read_outcome
            .provider_message()
            .is_some_and(|message| message.contains("larger than 7 bytes"))
    );
}

// The fake-stream Invoke call every conversion failure in these cases comes through.
fn fake_invoke_failure(messages: &Json, reply: Reply) -> Outcome {
    let inputs = Inputs::new(reply);
    let mut call = invoke(BASE, DATA, messages);
    call.fake_stream = true;
    run(Call::Invoke(call), &inputs, Consume::None)
}

fn transport_refusals(messages: &Json) {
    let refused_inputs = Inputs::new(Reply::refusing(Refusal::ReadTimeout));
    let refused_outcome = run(
        Call::Invoke(invoke(BASE, DATA, messages)),
        &refused_inputs,
        Consume::None,
    );
    assert_eq!(refused_outcome.provider_status(), Some(408));

    let large_inputs = Inputs::new(Reply::refusing(Refusal::TooLarge(7)));
    let large_outcome = run(
        Call::Invoke(invoke(BASE, DATA, messages)),
        &large_inputs,
        Consume::None,
    );
    assert_eq!(large_outcome.provider_status(), Some(500));
    assert!(
        large_outcome
            .provider_message()
            .is_some_and(|message| message.contains("larger than 7 bytes"))
    );
}

fn converse_status_failure(messages: &Json) {
    // Converse has no catch: the status failure carries the bytes repr, and the others pass through.
    let mut converse_reply = Reply::buffered("{\"message\":\"boom\"}\n");
    converse_reply.status = 500;
    converse_reply.headers = vec![("x-amzn-requestid".to_owned(), "req-1".to_owned())];
    let converse_inputs = Inputs::new(converse_reply);
    let converse_outcome = run(
        Call::Converse(converse(messages)),
        &converse_inputs,
        Consume::None,
    );
    assert_eq!(converse_outcome.provider_status(), Some(500));
    assert_eq!(
        converse_outcome.provider_message(),
        Some(r#"b'{"message":"boom"}\n'"#)
    );
    assert!(
        matches!(converse_outcome.provider_body(), Some(ResponseBody::Read(text)) if text == "{\"message\":\"boom\"}\n")
    );
    let error = converse_outcome.provider().unwrap();
    assert_eq!(
        error.response_headers,
        vec![("x-amzn-requestid".to_owned(), "req-1".to_owned())]
    );
}

fn converse_bytes_repr(messages: &Json) {
    // Python's bytes repr escapes the delimiter, a backslash, controls and non-ASCII bytes.
    let bytes_inputs = Inputs::new(Reply {
        status: 500,
        headers: Vec::new(),
        reads: vec![Ok(b"it's \"x\" \\\t\r\xff".to_vec())],
        refused: None,
    });
    let bytes_outcome = run(
        Call::Converse(converse(messages)),
        &bytes_inputs,
        Consume::None,
    );
    assert_eq!(bytes_outcome.provider_status(), Some(500));
    assert_eq!(
        bytes_outcome.provider_message(),
        Some(r#"b'it\'s "x" \\\t\r\xff'"#)
    );

    // A body with a single quote and no double quote takes double quotes, as Python's repr does.
    let apostrophe_inputs = Inputs::new(Reply {
        status: 500,
        headers: Vec::new(),
        reads: vec![Ok(b"it's".to_vec())],
        refused: None,
    });
    let apostrophe_outcome = run(
        Call::Converse(converse(messages)),
        &apostrophe_inputs,
        Consume::None,
    );
    assert_eq!(apostrophe_outcome.provider_status(), Some(500));
    assert_eq!(apostrophe_outcome.provider_message(), Some(r#"b"it's""#));
}

fn converse_passthrough_failures(messages: &Json) {
    let transport_inputs = Inputs::new(Reply::refusing(Refusal::ReadTimeout));
    let transport_outcome = run(
        Call::Converse(converse(messages)),
        &transport_inputs,
        Consume::None,
    );
    assert!(matches!(
        transport_outcome.error,
        Some(HttpStreamError::Transport(_))
    ));

    let body_reply = Reply {
        status: 400,
        headers: Vec::new(),
        reads: vec![Err(HttpError::Timeout)],
        refused: None,
    };
    let body_inputs = Inputs::new(body_reply);
    let body_outcome = run(
        Call::Converse(converse(messages)),
        &body_inputs,
        Consume::None,
    );
    assert!(matches!(body_outcome.error, Some(HttpStreamError::Body(_))));
}

fn case_3_converse_stream_headers() {
    let messages = messages();
    converse_stream_headers(&messages);
    converse_client_selection(&messages);
    converse_tally_failures(&messages);
    converse_json_mode(&messages);
}

fn converse_stream_headers(messages: &Json) {
    let mut reply = Reply::frames(&[START, HELLO, WORLD, STOP, USAGE]);
    reply
        .headers
        .push(("x-amzn-requestid".to_owned(), "req-1".to_owned()));
    let expected_headers = {
        let mut pairs = reply.headers.clone();
        pairs.sort();
        pairs
    };
    let inputs = Inputs::new(reply);
    let outcome = run(Call::Converse(converse(messages)), &inputs, Consume::All);
    assert!(outcome.error.is_none());
    assert!(outcome.failures.is_empty());
    assert_eq!(
        sorted_pairs(outcome.headers.as_ref().unwrap()),
        expected_headers
    );
    assert_eq!(content_of(&outcome.chunks).concat(), "hello world");
    assert_stream_metadata(&outcome);
    assert_converse_posts(&inputs, &outcome);
}

fn assert_stream_metadata(outcome: &Outcome) {
    // Every event chunk carries the model the call passed, which the decoder stores at construction.
    let models: Vec<&str> = outcome
        .chunks
        .iter()
        .filter_map(|chunk| chunk_field(chunk, Field::Model))
        .collect();
    assert_eq!(models.len(), outcome.chunks.len());
    assert!(
        models
            .iter()
            .all(|model| *model == "bedrock/anthropic.claude-3")
    );
    // Only the stop event names a reason; the usage event carries the totals instead.
    let reasons: Vec<&str> = outcome
        .chunks
        .iter()
        .filter_map(|chunk| chunk_field(chunk, Field::FinishReason))
        .filter(|reason| !reason.is_empty())
        .collect();
    assert_eq!(reasons, vec!["stop"]);
}

fn assert_converse_posts(inputs: &Inputs, outcome: &Outcome) {
    assert_eq!(outcome.chunks.len(), 5);
    assert_eq!(inputs.posts()[0].body, DATA);
    assert!(inputs.posts()[0].stream);
    assert_eq!(outcome.logs.len(), 1);
}

fn converse_client_selection(messages: &Json) {
    // A supplied client wins over the default handler here too.
    let supplied = Inputs::new(Reply::frames(&[FIRST]));
    let default = Inputs::new(Reply::refusing(Refusal::Closed));
    let supplied_outcome = run_supplied(
        Call::Converse(converse(messages)),
        &supplied,
        &default,
        Consume::All,
    );
    assert!(supplied_outcome.error.is_none());
    assert_eq!(supplied.calls(), 1);
    assert_eq!(default.calls(), 0);
    assert_eq!(supplied_outcome.chunks.len(), 1);

    // With no supplied client the default handler serves the call.
    let default_inputs = Inputs::new(Reply::frames(&[FIRST]));
    let default_outcome = run_default(
        Call::Converse(converse(messages)),
        &default_inputs,
        Consume::All,
    );
    assert!(default_outcome.error.is_none());
    assert_eq!(default_inputs.calls(), 1);
    assert_eq!(default_outcome.chunks.len(), 1);
}

fn converse_tally_failures(messages: &Json) {
    // An event stream that decodes to no events fails through the tally with the 502 LiteLLM raises.
    let mut empty_reply = Reply::frames(&[]);
    empty_reply
        .headers
        .push(("x-amzn-requestid".to_owned(), "req-1".to_owned()));
    let empty_inputs = Inputs::new(empty_reply);
    let empty_outcome = run(
        Call::Converse(converse(messages)),
        &empty_inputs,
        Consume::All,
    );
    assert_eq!(empty_outcome.failure_statuses(), vec![Some(502)]);
    // The 502 names the headers the response carried, which the tally was given when the stream was built.
    let message = failure_message(&empty_outcome.failures[0]).expect("the tally names the headers");
    assert!(message.contains("content-type='application/vnd.amazon.eventstream'"));
    assert!(message.contains("x-amzn-requestid='req-1'"));

    // A body too short to hold a frame is still no events at EOF, so the tally again.
    let short_inputs = Inputs::new(Reply::frames(&["6e6f742061206672616d65"]));
    let short_outcome = run(
        Call::Converse(converse(messages)),
        &short_inputs,
        Consume::All,
    );
    assert_eq!(short_outcome.failure_statuses(), vec![Some(502)]);

    // A malformed frame fails as it is read, without a status of its own.
    let malformed_inputs = Inputs::new(Reply::frames(&[MALFORMED]));
    let malformed_outcome = run(
        Call::Converse(converse(messages)),
        &malformed_inputs,
        Consume::All,
    );
    assert_eq!(malformed_outcome.failure_statuses(), vec![None]);
}

fn converse_json_mode(messages: &Json) {
    // json_mode makes the base decoder read the synthetic response-format tool as content.
    let json_inputs = Inputs::new(Reply::frames(&[
        JSON_TOOL_START,
        JSON_TOOL_DELTA,
        JSON_TOOL_STOP,
    ]));
    let mut json_call = converse(messages);
    json_call.json_mode = Some(true);
    let json_outcome = run(Call::Converse(json_call), &json_inputs, Consume::All);
    assert!(json_outcome.error.is_none());
    assert!(json_outcome.failures.is_empty());
    let contents = content_of(&json_outcome.chunks);
    assert!(
        contents
            .iter()
            .any(|content| *content == r#"{"sign": "WZPZ"}"#)
    );

    // Without json_mode the same events keep the tool as a tool call and no content.
    let plain_inputs = Inputs::new(Reply::frames(&[
        JSON_TOOL_START,
        JSON_TOOL_DELTA,
        JSON_TOOL_STOP,
    ]));
    let plain_outcome = run(
        Call::Converse(converse(messages)),
        &plain_inputs,
        Consume::All,
    );
    let plain_contents = content_of(&plain_outcome.chunks);
    assert!(
        !plain_contents
            .iter()
            .any(|content| *content == r#"{"sign": "WZPZ"}"#)
    );
}

fn case_4_fake_stream_projection() {
    let messages = messages();
    fake_invoke_projection(&messages);
    fake_tool_projection(&messages);
    fake_client_paths(&messages);
    provider_decoder(&messages);
    buffered_json_tool(&messages);
    buffered_conversion_failures(&messages);
}

fn fake_invoke_projection(messages: &Json) {
    let inputs = Inputs::new(Reply::buffered(REPLY));
    let mut call = invoke(BASE, DATA, messages);
    call.fake_stream = true;
    let outcome = run(Call::Invoke(call), &inputs, Consume::All);
    assert!(outcome.error.is_none());
    assert_eq!(outcome.chunks.len(), 1);
    let chunk = generic(&outcome.chunks[0]);
    assert_eq!(chunk.text, "hello");
    assert!(chunk.tool_use.is_none());
    assert!(chunk.is_finished);
    assert_eq!(chunk.finish_reason, "stop");
    assert_eq!(chunk.index, Some(0));
    let usage = chunk.usage.as_ref().unwrap();
    assert_eq!(
        (
            usage.prompt_tokens,
            usage.completion_tokens,
            usage.total_tokens
        ),
        (10, 5, 15)
    );
}

fn fake_tool_projection(messages: &Json) {
    // The buffered converter never sees json_mode; the iterator applies it to the tool call afterwards.
    let tool_inputs = Inputs::new(Reply::buffered(TOOL_REPLY));
    let mut tool_call = invoke(BASE, DATA, messages);
    tool_call.fake_stream = true;
    let tool_outcome = run(Call::Invoke(tool_call), &tool_inputs, Consume::All);
    let tool_chunk = generic(&tool_outcome.chunks[0]);
    assert_eq!(tool_chunk.text, "");
    let tool = tool_chunk.tool_use.as_ref().unwrap();
    assert_eq!(tool_member(tool, "id").as_str(), Some("tool-1"));
    assert_eq!(tool_member(tool, "type").as_str(), Some("function"));
    let function = tool_member(tool, "function");
    assert_eq!(tool_member(function, "name").as_str(), Some("top_song"));
    assert_eq!(
        tool_member(function, "arguments").as_str(),
        Some(r#"{"sign": "WZPZ"}"#)
    );
    assert_eq!(tool_member(tool, "index").as_i64(), Some(0));

    let json_inputs = Inputs::new(Reply::buffered(TOOL_REPLY));
    let mut json_call = invoke(BASE, DATA, messages);
    json_call.fake_stream = true;
    json_call.json_mode = Some(true);
    let json_outcome = run(Call::Invoke(json_call), &json_inputs, Consume::All);
    let json_chunk = generic(&json_outcome.chunks[0]);
    assert_eq!(json_chunk.text, r#"{"sign": "WZPZ"}"#);
    assert!(json_chunk.tool_use.is_none());
}

fn fake_client_paths(messages: &Json) {
    // The sync Invoke helper and the Converse helper convert the same body the same way.
    let sync_inputs = Inputs::new(Reply::buffered(REPLY));
    let mut sync_call = invoke(BASE, DATA, messages);
    sync_call.fake_stream = true;
    sync_call.sync = true;
    let sync_outcome = run(Call::Invoke(sync_call), &sync_inputs, Consume::All);
    assert_eq!(generic(&sync_outcome.chunks[0]).text, "hello");

    let converse_inputs = Inputs::new(Reply::buffered(REPLY));
    let mut converse_call = converse(messages);
    converse_call.fake_stream = true;
    let converse_outcome = run(
        Call::Converse(converse_call),
        &converse_inputs,
        Consume::All,
    );
    assert_eq!(generic(&converse_outcome.chunks[0]).text, "hello");
}

fn provider_decoder(messages: &Json) {
    // A named provider selects its own decoder instead of the base projection.
    let provider_inputs = Inputs::new(Reply::frames(&[DEEPSEEK_CHUNK]));
    let mut provider_call = invoke(BASE, DATA, messages);
    provider_call.provider = Some("deepseek_r1");
    let provider_outcome = run(Call::Invoke(provider_call), &provider_inputs, Consume::All);
    assert!(provider_outcome.error.is_none());
    assert!(provider_outcome.failures.is_empty());
    assert!(matches!(
        &provider_outcome.chunks[0],
        InvokeChunk::DeepSeek(_)
    ));
}

fn buffered_json_tool(messages: &Json) {
    // json_mode is not the buffered converter's: there the synthetic tool stays a tool call.
    let synthetic_inputs = Inputs::new(Reply::buffered(JSON_TOOL_REPLY));
    let mut synthetic_call = invoke(BASE, DATA, messages);
    synthetic_call.fake_stream = true;
    synthetic_call.json_mode = Some(true);
    let synthetic_outcome = run(
        Call::Invoke(synthetic_call),
        &synthetic_inputs,
        Consume::All,
    );
    let synthetic_chunk = generic(&synthetic_outcome.chunks[0]);
    assert_eq!(synthetic_chunk.text, r#"{"sign": "WZPZ"}"#);
    assert!(synthetic_chunk.tool_use.is_none());
    assert_eq!(synthetic_chunk.finish_reason, "tool_calls");

    let synthetic_converse_inputs = Inputs::new(Reply::buffered(JSON_TOOL_REPLY));
    let mut synthetic_converse_call = converse(messages);
    synthetic_converse_call.fake_stream = true;
    synthetic_converse_call.json_mode = Some(true);
    let synthetic_converse_outcome = run(
        Call::Converse(synthetic_converse_call),
        &synthetic_converse_inputs,
        Consume::All,
    );
    let synthetic_converse_chunk = generic(&synthetic_converse_outcome.chunks[0]);
    assert_eq!(synthetic_converse_chunk.text, r#"{"sign": "WZPZ"}"#);
    assert_eq!(synthetic_converse_chunk.finish_reason, "tool_calls");
}

fn buffered_conversion_failures(messages: &Json) {
    // A body the converter cannot read is the 422 it raises; a missing output member is the catch-all's 500.
    let bad_outcome = fake_invoke_failure(messages, Reply::buffered("[]"));
    assert_eq!(bad_outcome.provider_status(), Some(422));
    assert!(
        bad_outcome
            .provider_message()
            .is_some_and(|message| message.starts_with("Error converting to valid response block="))
    );

    let missing_outcome = fake_invoke_failure(messages, Reply::buffered("{}"));
    assert_eq!(missing_outcome.provider_status(), Some(500));
}

fn case_5_post_call_before_consumption() {
    let messages = messages();
    invoke_logs_before_read(&messages);
    empty_stream_logs(&messages);
    fake_stream_logs_twice(&messages);
    converse_logs_before_read(&messages);
}

fn invoke_logs_before_read(messages: &Json) {
    let inputs = Inputs::new(Reply::frames(&[FIRST]));
    let outcome = run(
        Call::Invoke(invoke(BASE, DATA, messages)),
        &inputs,
        Consume::None,
    );
    assert!(outcome.error.is_none());
    assert_eq!(inputs.polls(), 0);
    assert_eq!(outcome.logs.len(), 1);
    let logged = outcome.logged(0);
    assert_eq!(member(logged, "input"), messages);
    assert_eq!(
        member(logged, "original_response"),
        &Json::Text("first stream response received".to_owned())
    );
    assert_eq!(
        member(member(logged, "additional_args"), "complete_input_dict"),
        &Json::Text(DATA.to_owned())
    );
}

fn empty_stream_logs(messages: &Json) {
    // An empty stream is logged before the tally reports it, and the consumer sees the 502.
    let empty_inputs = Inputs::new(Reply::frames(&[]));
    let empty_outcome = run(
        Call::Invoke(invoke(BASE, DATA, messages)),
        &empty_inputs,
        Consume::All,
    );
    assert_eq!(empty_outcome.logs.len(), 1);
    assert_eq!(empty_outcome.failure_statuses(), vec![Some(502)]);

    let short_inputs = Inputs::new(Reply::frames(&["6e6f742061206672616d65"]));
    let short_outcome = run(
        Call::Invoke(invoke(BASE, DATA, messages)),
        &short_inputs,
        Consume::All,
    );
    assert_eq!(short_outcome.logs.len(), 1);
    assert_eq!(short_outcome.failure_statuses(), vec![Some(502)]);
    assert!(matches!(
        &short_outcome.failures[0],
        StreamFailure::Decode(_)
    ));
}

fn fake_stream_logs_twice(messages: &Json) {
    // A fake stream logs the buffered response first and the opened stream second.
    let fake_inputs = Inputs::new(Reply::buffered(REPLY));
    let mut fake_call = invoke(BASE, DATA, messages);
    fake_call.fake_stream = true;
    let fake_outcome = run(Call::Invoke(fake_call), &fake_inputs, Consume::All);
    assert_eq!(fake_outcome.logs.len(), 2);
    let buffered = fake_outcome.logged(0);
    assert_eq!(
        member(buffered, "original_response"),
        &Json::Text(REPLY.to_owned())
    );
    assert_eq!(member(buffered, "input"), messages);
    assert_eq!(
        member(member(buffered, "additional_args"), "complete_input_dict"),
        &Json::Text(DATA.to_owned())
    );
    let opened = fake_outcome.logged(1);
    assert_eq!(
        member(opened, "original_response"),
        &Json::Text("first stream response received".to_owned())
    );
}

fn converse_logs_before_read(messages: &Json) {
    let converse_inputs = Inputs::new(Reply::frames(&[FIRST]));
    let converse_outcome = run(
        Call::Converse(converse(messages)),
        &converse_inputs,
        Consume::None,
    );
    assert_eq!(converse_inputs.polls(), 0);
    assert_eq!(converse_outcome.logs.len(), 1);
    assert_eq!(
        member(converse_outcome.logged(0), "original_response"),
        &Json::Text("first stream response received".to_owned())
    );
}
