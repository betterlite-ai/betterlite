use betterlite_http::error::HttpError;
use betterlite_streaming::stream_chunk_shaping::records::GenericChunk;
use rstest::rstest;
use serde_json::{Value, json, to_value};

use super::support::{
    BASE, Call, Consume, DATA, FIRST, Inputs, Outcome, REPLY, Refusal, Reply, converse, invoke,
    messages, run, run_default, run_supplied, sorted_pairs,
};
use crate::bedrock_http_stream::error::HttpStreamError;
use crate::bedrock_http_stream::stream::StreamFailure;
use crate::bedrock_settings::ResponseBody;
use crate::converse_events::ChunkProjection;
use crate::invoke_events::InvokeChunk;

const START: &str = "0000008e0000002ab04f184c0b3a6576656e742d747970650700056368756e6b0d3a6d6573736167652d747970650700056576656e747b226279746573223a2265794a7a644746796443493665794a796232786c496a6f6959584e7a61584e3059573530496e3073496d4e76626e526c626e52436247396a61306c755a475634496a6f7766513d3d227df9b9a0e5";
const HELLO: &str = "000000860000002a803f538d0b3a6576656e742d747970650700056368756e6b0d3a6d6573736167652d747970650700056576656e747b226279746573223a2265794a6b5a5778305953493665794a305a586830496a6f69614756736247386966537769593239756447567564454a7362324e725357356b5a5867694f6a4239227d23df9631";
const WORLD: &str = "0000008a0000002a45cfbe8c0b3a6576656e742d747970650700056368756e6b0d3a6d6573736167652d747970650700056576656e747b226279746573223a2265794a6b5a5778305953493665794a305a586830496a6f6949486476636d786b496e3073496d4e76626e526c626e52436247396a61306c755a475634496a6f7766513d3d227d3fd85d87";
const STOP: &str = "0000006a0000002a6d1b79530b3a6576656e742d747970650700056368756e6b0d3a6d6573736167652d747970650700056576656e747b226279746573223a2265794a7a64473977556d566863323975496a6f695a57356b58335231636d346966513d3d227d7210a35f";
const USAGE: &str = "000000be0000002a116ea0ca0b3a6576656e742d747970650700056368756e6b0d3a6d6573736167652d747970650700056576656e747b226279746573223a2265794a745a58527961574e7a496a7037496d78686447567559336c4e637949364f583073496e567a5957646c496a7037496d6c7563485630564739725a57357a496a6f784d4377696233563063485630564739725a57357a496a6f314c434a30623352686246527661325675637949364d54563966513d3d227da255de78";
// Sixteen zero bytes: enough for a prelude, whose checksum cannot match.
const MALFORMED: &str = "00000000000000000000000000000000";
const FIRST: &str = "000000660000002aa8eb94520b3a6576656e742d747970650700056368756e6b0d3a6d6573736167652d747970650700056576656e747b226279746573223a2265794a7664585277645852555a586830496a6f695a6d6c796333516966513d3d227d0bc3a082";

const TOOL_REPLY: &str = r#"{"output":{"message":{"role":"assistant","content":[{"toolUse":{"toolUseId":"tool-1","name":"top_song","input":{"sign":"WZPZ"}}}]}},"stopReason":"tool_use","usage":{"inputTokens":2,"outputTokens":3,"totalTokens":5}}"#;

fn entry(outcome: &Outcome, index: usize) -> Value {
    to_value(outcome.logged(index)).unwrap()
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

fn content(chunk: &InvokeChunk) -> Option<&str> {
    match chunk {
        InvokeChunk::Base(projection) => match projection.as_ref() {
            ChunkProjection::Converse(converse) => converse.delta.content.as_deref(),
            ChunkProjection::Generic(generic) => Some(generic.text.as_str()),
        },
        InvokeChunk::Anthropic(_) | InvokeChunk::DeepSeek(_) | InvokeChunk::OpenAiCompatible(_) => {
            None
        }
    }
}

fn finish_reason(chunk: &InvokeChunk) -> Option<&str> {
    match chunk {
        InvokeChunk::Base(projection) => match projection.as_ref() {
            ChunkProjection::Converse(converse) => converse.finish_reason.as_deref(),
            ChunkProjection::Generic(generic) => Some(generic.finish_reason.as_str()),
        },
        InvokeChunk::Anthropic(_) | InvokeChunk::DeepSeek(_) | InvokeChunk::OpenAiCompatible(_) => {
            None
        }
    }
}

#[rstest]
#[case::invoke_stream_url(invoke_stream_url)]
#[case::http_error_before_frame(http_error_before_frame)]
#[case::converse_stream_headers(converse_stream_headers)]
#[case::fake_stream_projection(fake_stream_projection)]
#[case::post_call_before_consumption(post_call_before_consumption)]
fn bedrock_http_stream_cases(#[case] case: fn()) {
    case();
}

fn invoke_stream_url() {
    let messages = messages();
    let api_base = "https://bedrock-runtime.us-east-1.amazonaws.com/custom/invoke?trace=1";
    let inputs = Inputs::new(Reply::frames(&[FIRST]));
    let outcome = run(
        Call::Invoke(invoke(api_base, DATA, &messages)),
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
    let inputs = Inputs::new(Reply::frames(&[FIRST]));
    let outcome = run_default(
        Call::Invoke(invoke(api_base, DATA, &messages)),
        &inputs,
        Consume::None,
    );
    assert!(outcome.error.is_none());
    assert_eq!(inputs.calls(), 1);
    assert_eq!(inputs.posts()[0].url, api_base);

    // A supplied client wins over the default handler, which then sees no request.
    let supplied = Inputs::new(Reply::frames(&[FIRST]));
    let default = Inputs::new(Reply::refusing(Refusal::Closed));
    let outcome = run_supplied(
        Call::Invoke(invoke(api_base, DATA, &messages)),
        &supplied,
        &default,
        Consume::None,
    );
    assert!(outcome.error.is_none());
    assert_eq!(supplied.calls(), 1);
    assert_eq!(default.calls(), 0);

    // The async helper never reads a signed body, even one a caller supplied.
    let inputs = Inputs::new(Reply::frames(&[FIRST]));
    let mut call = invoke(api_base, DATA, &messages);
    call.signed_json_body = Some(b"signed-async");
    let outcome = run(Call::Invoke(call), &inputs, Consume::None);
    assert!(outcome.error.is_none());
    assert_eq!(inputs.posts()[0].body, DATA);

    // The sync helper sends the signed body whenever one exists, empty bytes included.
    let inputs = Inputs::new(Reply::frames(&[FIRST]));
    let mut call = invoke(api_base, DATA, &messages);
    call.sync = true;
    call.signed_json_body = Some(b"signed-json");
    let outcome = run(Call::Invoke(call), &inputs, Consume::None);
    assert!(outcome.error.is_none());
    assert_eq!(inputs.posts()[0].body, "signed-json");

    let inputs = Inputs::new(Reply::frames(&[FIRST]));
    let mut call = invoke(api_base, DATA, &messages);
    call.sync = true;
    call.signed_json_body = Some(b"");
    let outcome = run(Call::Invoke(call), &inputs, Consume::None);
    assert!(outcome.error.is_none());
    assert_eq!(inputs.posts()[0].body, "");

    let inputs = Inputs::new(Reply::frames(&[FIRST]));
    let mut call = invoke(api_base, DATA, &messages);
    call.sync = true;
    let outcome = run(Call::Invoke(call), &inputs, Consume::None);
    assert!(outcome.error.is_none());
    assert_eq!(inputs.posts()[0].body, DATA);

    // A fake stream tells the client not to stream, on every helper.
    let inputs = Inputs::new(Reply::buffered(REPLY));
    let mut call = invoke(api_base, DATA, &messages);
    call.fake_stream = true;
    let outcome = run(Call::Invoke(call), &inputs, Consume::None);
    assert!(outcome.error.is_none());
    assert!(!inputs.posts()[0].stream);

    let inputs = Inputs::new(Reply::buffered(REPLY));
    let mut call = converse(api_base, DATA, &messages);
    call.fake_stream = true;
    let outcome = run(Call::Converse(call), &inputs, Consume::None);
    assert!(outcome.error.is_none());
    assert!(!inputs.posts()[0].stream);
}

fn http_error_before_frame() {
    let messages = messages();

    let mut reply = Reply::streaming(&["ignored"]);
    reply.status = 400;
    let inputs = Inputs::new(reply);
    let outcome = run(
        Call::Invoke(invoke(BASE, DATA, &messages)),
        &inputs,
        Consume::None,
    );
    assert_eq!(outcome.provider_status(), Some(400));
    assert_eq!(outcome.provider_message(), Some("Bad Request"));
    assert!(matches!(
        outcome.provider_body(),
        Some(ResponseBody::Unread { reason_phrase }) if reason_phrase == "Bad Request"
    ));
    assert_eq!(inputs.polls(), 0);
    assert!(outcome.logs.is_empty());

    for (status, reason) in [(201, "Created"), (204, "No Content")] {
        let mut reply = Reply::streaming(&[]);
        reply.status = status;
        let inputs = Inputs::new(reply);
        let outcome = run(
            Call::Invoke(invoke(BASE, DATA, &messages)),
            &inputs,
            Consume::None,
        );
        assert_eq!(outcome.provider_status(), Some(status));
        assert_eq!(outcome.provider_message(), Some(reason));
    }

    // A fake stream has read the body, so the failure carries its text.
    let mut reply = Reply::buffered("boom");
    reply.status = 500;
    let inputs = Inputs::new(reply);
    let mut call = invoke(BASE, DATA, &messages);
    call.fake_stream = true;
    let outcome = run(Call::Invoke(call), &inputs, Consume::None);
    assert_eq!(outcome.provider_status(), Some(500));
    assert_eq!(outcome.provider_message(), Some("boom"));
    assert!(matches!(outcome.provider_body(), Some(ResponseBody::Read(text)) if text == "boom"));

    let reply = Reply {
        status: 500,
        headers: Vec::new(),
        reads: vec![Err(HttpError::Timeout)],
        refused: None,
    };
    let inputs = Inputs::new(reply);
    let mut call = invoke(BASE, DATA, &messages);
    call.fake_stream = true;
    let outcome = run(Call::Invoke(call), &inputs, Consume::None);
    assert_eq!(outcome.provider_status(), Some(408));
    assert_eq!(outcome.provider_message(), Some("Timeout error occurred."));

    let inputs = Inputs::new(Reply::refusing(Refusal::ReadTimeout));
    let outcome = run(
        Call::Invoke(invoke(BASE, DATA, &messages)),
        &inputs,
        Consume::None,
    );
    assert_eq!(outcome.provider_status(), Some(408));

    let inputs = Inputs::new(Reply::refusing(Refusal::TooLarge(7)));
    let outcome = run(
        Call::Invoke(invoke(BASE, DATA, &messages)),
        &inputs,
        Consume::None,
    );
    assert_eq!(outcome.provider_status(), Some(500));
    assert!(
        outcome
            .provider_message()
            .is_some_and(|message| message.contains("larger than 7 bytes"))
    );

    // Converse has no catch: the status failure carries the bytes repr, and the others pass through.
    let mut reply = Reply::buffered("{\"message\":\"boom\"}\n");
    reply.status = 500;
    reply.headers = vec![("x-amzn-requestid".to_owned(), "req-1".to_owned())];
    let inputs = Inputs::new(reply);
    let outcome = run(
        Call::Converse(converse(BASE, DATA, &messages)),
        &inputs,
        Consume::None,
    );
    assert_eq!(outcome.provider_status(), Some(500));
    assert_eq!(
        outcome.provider_message(),
        Some(r#"b'{"message":"boom"}\n'"#)
    );
    assert!(
        matches!(outcome.provider_body(), Some(ResponseBody::Read(text)) if text == "{\"message\":\"boom\"}\n")
    );
    let error = outcome.provider().unwrap();
    assert_eq!(
        error.response_headers,
        vec![("x-amzn-requestid".to_owned(), "req-1".to_owned())]
    );

    let inputs = Inputs::new(Reply::refusing(Refusal::ReadTimeout));
    let outcome = run(
        Call::Converse(converse(BASE, DATA, &messages)),
        &inputs,
        Consume::None,
    );
    assert!(matches!(outcome.error, Some(HttpStreamError::Transport(_))));

    let reply = Reply {
        status: 400,
        headers: Vec::new(),
        reads: vec![Err(HttpError::Timeout)],
        refused: None,
    };
    let inputs = Inputs::new(reply);
    let outcome = run(
        Call::Converse(converse(BASE, DATA, &messages)),
        &inputs,
        Consume::None,
    );
    assert!(matches!(outcome.error, Some(HttpStreamError::Body(_))));
}

fn converse_stream_headers() {
    let messages = messages();
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
    let outcome = run(
        Call::Converse(converse(BASE, DATA, &messages)),
        &inputs,
        Consume::All,
    );
    assert!(outcome.error.is_none());
    assert!(outcome.failures.is_empty());
    assert_eq!(
        sorted_pairs(outcome.headers.as_ref().unwrap()),
        expected_headers
    );
    let text: String = outcome.chunks.iter().filter_map(content).collect();
    assert_eq!(text, "hello world");
    // Only the stop event names a reason; the usage event carries the totals instead.
    let reasons: Vec<&str> = outcome
        .chunks
        .iter()
        .filter_map(finish_reason)
        .filter(|reason| !reason.is_empty())
        .collect();
    assert_eq!(reasons, vec!["stop"]);
    assert_eq!(outcome.chunks.len(), 5);
    assert_eq!(inputs.posts()[0].body, DATA);
    assert!(inputs.posts()[0].stream);
    assert_eq!(outcome.logs.len(), 1);

    // A supplied client wins over the default handler here too.
    let supplied = Inputs::new(Reply::frames(&[FIRST]));
    let default = Inputs::new(Reply::refusing(Refusal::Closed));
    let outcome = run_supplied(
        Call::Converse(converse(BASE, DATA, &messages)),
        &supplied,
        &default,
        Consume::All,
    );
    assert!(outcome.error.is_none());
    assert_eq!(supplied.calls(), 1);
    assert_eq!(default.calls(), 0);
    assert_eq!(outcome.chunks.len(), 1);

    // With no supplied client the default handler serves the call.
    let inputs = Inputs::new(Reply::frames(&[FIRST]));
    let outcome = run_default(
        Call::Converse(converse(BASE, DATA, &messages)),
        &inputs,
        Consume::All,
    );
    assert!(outcome.error.is_none());
    assert_eq!(inputs.calls(), 1);
    assert_eq!(outcome.chunks.len(), 1);

    // An event stream that decodes to no events fails through the tally with the 502 LiteLLM raises.
    let inputs = Inputs::new(Reply::streaming(&[]));
    let outcome = run(
        Call::Converse(converse(BASE, DATA, &messages)),
        &inputs,
        Consume::All,
    );
    assert_eq!(outcome.failure_statuses(), vec![Some(502)]);

    // A body too short to hold a frame is still no events at EOF, so the tally again.
    let inputs = Inputs::new(Reply::streaming(&["not a frame"]));
    let outcome = run(
        Call::Converse(converse(BASE, DATA, &messages)),
        &inputs,
        Consume::All,
    );
    assert_eq!(outcome.failure_statuses(), vec![Some(502)]);

    // A malformed frame fails as it is read, without a status of its own.
    let inputs = Inputs::new(Reply::frames(&[MALFORMED]));
    let outcome = run(
        Call::Converse(converse(BASE, DATA, &messages)),
        &inputs,
        Consume::All,
    );
    assert_eq!(outcome.failure_statuses(), vec![None]);
}

fn fake_stream_projection() {
    let messages = messages();

    let inputs = Inputs::new(Reply::buffered(REPLY));
    let mut call = invoke(BASE, DATA, &messages);
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

    // The buffered converter never sees json_mode; the iterator applies it to the tool call afterwards.
    let inputs = Inputs::new(Reply::buffered(TOOL_REPLY));
    let mut call = invoke(BASE, DATA, &messages);
    call.fake_stream = true;
    let outcome = run(Call::Invoke(call), &inputs, Consume::All);
    let chunk = generic(&outcome.chunks[0]);
    assert_eq!(chunk.text, "");
    let tool = chunk.tool_use.as_ref().unwrap();
    assert_eq!(tool["id"], json!("tool-1"));
    assert_eq!(tool["type"], json!("function"));
    assert_eq!(tool["function"]["name"], json!("top_song"));
    assert_eq!(tool["function"]["arguments"], json!(r#"{"sign": "WZPZ"}"#));
    assert_eq!(tool["index"], json!(0));

    let inputs = Inputs::new(Reply::buffered(TOOL_REPLY));
    let mut call = invoke(BASE, DATA, &messages);
    call.fake_stream = true;
    call.json_mode = Some(true);
    let outcome = run(Call::Invoke(call), &inputs, Consume::All);
    let chunk = generic(&outcome.chunks[0]);
    assert_eq!(chunk.text, r#"{"sign": "WZPZ"}"#);
    assert!(chunk.tool_use.is_none());

    // The sync Invoke helper and the Converse helper convert the same body the same way.
    let inputs = Inputs::new(Reply::buffered(REPLY));
    let mut call = invoke(BASE, DATA, &messages);
    call.fake_stream = true;
    call.sync = true;
    let outcome = run(Call::Invoke(call), &inputs, Consume::All);
    assert_eq!(generic(&outcome.chunks[0]).text, "hello");

    let inputs = Inputs::new(Reply::buffered(REPLY));
    let mut call = converse(BASE, DATA, &messages);
    call.fake_stream = true;
    let outcome = run(Call::Converse(call), &inputs, Consume::All);
    assert_eq!(generic(&outcome.chunks[0]).text, "hello");

    // A body the converter cannot read is the 422 it raises; a missing output member is the catch-all's 500.
    let inputs = Inputs::new(Reply::buffered("[]"));
    let mut call = invoke(BASE, DATA, &messages);
    call.fake_stream = true;
    let outcome = run(Call::Invoke(call), &inputs, Consume::None);
    assert_eq!(outcome.provider_status(), Some(422));
    assert!(
        outcome
            .provider_message()
            .is_some_and(|message| message.starts_with("Error converting to valid response block="))
    );

    let inputs = Inputs::new(Reply::buffered("{}"));
    let mut call = invoke(BASE, DATA, &messages);
    call.fake_stream = true;
    let outcome = run(Call::Invoke(call), &inputs, Consume::None);
    assert_eq!(outcome.provider_status(), Some(500));
}

fn post_call_before_consumption() {
    let messages = messages();

    let inputs = Inputs::new(Reply::frames(&[FIRST]));
    let outcome = run(
        Call::Invoke(invoke(BASE, DATA, &messages)),
        &inputs,
        Consume::None,
    );
    assert!(outcome.error.is_none());
    assert_eq!(inputs.polls(), 0);
    assert_eq!(outcome.logs.len(), 1);
    let logged = entry(&outcome, 0);
    assert_eq!(logged["input"], json!([{"role": "user", "content": "hi"}]));
    assert_eq!(
        logged["original_response"],
        json!("first stream response received")
    );
    assert_eq!(
        logged["additional_args"]["complete_input_dict"],
        json!(DATA)
    );

    // An empty stream is logged before the tally reports it, and the consumer sees the 502.
    let inputs = Inputs::new(Reply::streaming(&[]));
    let outcome = run(
        Call::Invoke(invoke(BASE, DATA, &messages)),
        &inputs,
        Consume::All,
    );
    assert_eq!(outcome.logs.len(), 1);
    assert_eq!(outcome.failure_statuses(), vec![Some(502)]);

    let inputs = Inputs::new(Reply::streaming(&["not a frame"]));
    let outcome = run(
        Call::Invoke(invoke(BASE, DATA, &messages)),
        &inputs,
        Consume::All,
    );
    assert_eq!(outcome.logs.len(), 1);
    assert_eq!(outcome.failure_statuses(), vec![Some(502)]);
    assert!(matches!(&outcome.failures[0], StreamFailure::Decode(_)));

    // A fake stream logs the buffered response first and the opened stream second.
    let inputs = Inputs::new(Reply::buffered(REPLY));
    let mut call = invoke(BASE, DATA, &messages);
    call.fake_stream = true;
    let outcome = run(Call::Invoke(call), &inputs, Consume::All);
    assert_eq!(outcome.logs.len(), 2);
    let buffered = entry(&outcome, 0);
    assert_eq!(buffered["original_response"], json!(REPLY));
    assert_eq!(
        buffered["input"],
        json!([{"role": "user", "content": "hi"}])
    );
    assert_eq!(
        buffered["additional_args"]["complete_input_dict"],
        json!(DATA)
    );
    let opened = entry(&outcome, 1);
    assert_eq!(
        opened["original_response"],
        json!("first stream response received")
    );

    let inputs = Inputs::new(Reply::frames(&[FIRST]));
    let outcome = run(
        Call::Converse(converse(BASE, DATA, &messages)),
        &inputs,
        Consume::None,
    );
    assert_eq!(inputs.polls(), 0);
    assert_eq!(outcome.logs.len(), 1);
    assert_eq!(
        entry(&outcome, 0)["original_response"],
        json!("first stream response received")
    );
}
