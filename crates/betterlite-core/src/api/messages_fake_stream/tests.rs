#![cfg(test)]

use rstest::rstest;

use super::value::Json;
use super::{MessagesFakeStream, MessagesFakeStreamError};

fn obj(pairs: Vec<(&str, Json)>) -> Json {
    let mut owned_pairs = Vec::with_capacity(pairs.len());
    for (key, value) in pairs {
        owned_pairs.push((key.to_owned(), value));
    }
    Json::Object(owned_pairs.into_iter().collect())
}

fn arr(items: Vec<Json>) -> Json {
    Json::Array(items)
}

fn str_val(value: &str) -> Json {
    Json::String(value.to_owned())
}

fn num(value: i64) -> Json {
    Json::from(value)
}

fn parse_frame(frame: &[u8]) -> (String, Json) {
    let text = String::from_utf8(frame.to_vec()).expect("frames are ascii");
    let (event_line, rest) = text.split_once('\n').expect("frame has an event line");
    let event = event_line
        .strip_prefix("event: ")
        .expect("frame starts with event:")
        .to_owned();
    let data_line = rest
        .strip_prefix("data: ")
        .expect("frame has a data line")
        .strip_suffix("\n\n")
        .expect("frame ends with a blank line");
    let data = serde_json::from_str(data_line).expect("data line is valid json");
    (event, data)
}

fn capture(response: &Json) -> Result<Vec<(String, Json, Vec<u8>)>, MessagesFakeStreamError> {
    Ok(MessagesFakeStream::new(response)?
        .map(|frame| {
            let (event, data) = parse_frame(&frame);
            (event, data, frame)
        })
        .collect())
}

fn find_block<'frames>(
    frames: &'frames [(String, Json, Vec<u8>)],
    event: &str,
    index: i64,
) -> &'frames Json {
    frames
        .iter()
        .find(|(name, data, _)| name == event && data.get("index") == Some(&Json::from(index)))
        .map_or_else(
            || panic!("missing {event} at index {index}"),
            |(_, data, _)| data,
        )
}

fn deltas_at(frames: &[(String, Json, Vec<u8>)], index: i64) -> Vec<Json> {
    frames
        .iter()
        .filter(|(name, data, _)| {
            name == "content_block_delta" && data.get("index") == Some(&Json::from(index))
        })
        .map(|(_, data, _)| data["delta"].clone())
        .collect()
}

#[test]
fn message_lifecycle() {
    let response = obj(vec![
        ("type", str_val("message")),
        ("id", str_val("msg_a")),
        ("model", str_val("claude-sonnet-4-20250514")),
        ("role", str_val("assistant")),
        (
            "content",
            arr(vec![obj(vec![
                ("type", str_val("text")),
                ("text", str_val("Hi")),
            ])]),
        ),
        (
            "usage",
            obj(vec![("input_tokens", num(5)), ("output_tokens", num(2))]),
        ),
        ("stop_reason", str_val("end_turn")),
        ("extra", num(7)),
    ]);
    let frames = capture(&response).expect("well-formed response builds frames");
    let bytes: Vec<&[u8]> = frames
        .iter()
        .map(|(_, _, frame)| frame.as_slice())
        .collect();
    assert_eq!(
        bytes,
        vec![
            b"event: message_start\ndata: {\"type\": \"message_start\", \"message\": {\"id\": \"msg_a\", \"type\": \"message\", \"role\": \"assistant\", \"model\": \"claude-sonnet-4-20250514\", \"content\": [], \"stop_reason\": null, \"stop_sequence\": null, \"usage\": {\"input_tokens\": 5, \"output_tokens\": 0}}}\n\n".as_slice(),
            b"event: content_block_start\ndata: {\"type\": \"content_block_start\", \"index\": 0, \"content_block\": {\"type\": \"text\", \"text\": \"\"}}\n\n".as_slice(),
            b"event: content_block_delta\ndata: {\"type\": \"content_block_delta\", \"index\": 0, \"delta\": {\"type\": \"text_delta\", \"text\": \"Hi\"}}\n\n".as_slice(),
            b"event: content_block_stop\ndata: {\"type\": \"content_block_stop\", \"index\": 0}\n\n".as_slice(),
            b"event: message_delta\ndata: {\"type\": \"message_delta\", \"delta\": {\"stop_reason\": \"end_turn\", \"stop_sequence\": null}, \"usage\": {\"output_tokens\": 2, \"input_tokens\": 5}}\n\n".as_slice(),
            b"event: message_stop\ndata: {\"type\": \"message_stop\", \"usage\": {\"input_tokens\": 5, \"output_tokens\": 2}}\n\n".as_slice(),
        ]
    );
}

#[rstest]
#[case::missing_response(
    obj(vec![]),
    Json::Null,
    str_val("assistant"),
    (
        obj(vec![("input_tokens", num(0)), ("output_tokens", num(0))]),
        obj(vec![("output_tokens", num(0))]),
        obj(vec![]),
    )
)]
#[case::usage_explicit_null(
    obj(vec![("usage", Json::Null)]),
    Json::Null,
    str_val("assistant"),
    (
        obj(vec![("input_tokens", num(0)), ("output_tokens", num(0))]),
        obj(vec![("output_tokens", num(0))]),
        obj(vec![]),
    )
)]
#[case::usage_empty_map(
    obj(vec![("usage", obj(vec![]))]),
    Json::Null,
    str_val("assistant"),
    (
        obj(vec![("input_tokens", num(0)), ("output_tokens", num(0))]),
        obj(vec![("output_tokens", num(0))]),
        obj(vec![]),
    )
)]
#[case::role_explicit_null(
    obj(vec![("role", Json::Null)]),
    Json::Null,
    Json::Null,
    (
        obj(vec![("input_tokens", num(0)), ("output_tokens", num(0))]),
        obj(vec![("output_tokens", num(0))]),
        obj(vec![]),
    )
)]
#[case::truthy_usage_with_zeros(
    obj(vec![(
        "usage",
        obj(vec![
            ("input_tokens", num(0)),
            ("output_tokens", num(0)),
            ("cache_creation_input_tokens", num(0)),
            ("cache_read_input_tokens", num(3)),
            ("extra", num(7)),
        ])
    )]),
    Json::Null,
    str_val("assistant"),
    (
        obj(vec![("input_tokens", num(0)), ("output_tokens", num(0))]),
        obj(vec![
            ("output_tokens", num(0)),
            ("input_tokens", num(0)),
            ("cache_creation_input_tokens", num(0)),
            ("cache_read_input_tokens", num(3)),
        ]),
        obj(vec![
            ("input_tokens", num(0)),
            ("output_tokens", num(0)),
            ("cache_creation_input_tokens", num(0)),
            ("cache_read_input_tokens", num(3)),
            ("extra", num(7)),
        ]),
    )
)]
#[case::truthy_usage_with_nulls(
    obj(vec![(
        "usage",
        obj(vec![
            ("input_tokens", Json::Null),
            ("output_tokens", Json::Null),
            ("cache_creation_input_tokens", Json::Null),
        ])
    )]),
    Json::Null,
    str_val("assistant"),
    (
        obj(vec![("input_tokens", Json::Null), ("output_tokens", num(0))]),
        obj(vec![("output_tokens", Json::Null)]),
        obj(vec![
            ("input_tokens", Json::Null),
            ("output_tokens", Json::Null),
            ("cache_creation_input_tokens", Json::Null),
        ]),
    )
)]
fn usage_projection(
    #[case] response: Json,
    #[case] expected_id: Json,
    #[case] expected_role: Json,
    #[case] usages: (Json, Json, Json),
) {
    let (start_usage, delta_usage, stop_usage) = usages;
    let frames = capture(&response).expect("usage fixture builds frames");
    assert_eq!(frames.len(), 3);
    let (_, start, _) = &frames[0];
    assert_eq!(start["message"]["id"], expected_id);
    assert_eq!(start["message"]["role"], expected_role);
    assert_eq!(start["message"]["usage"], start_usage);
    let (_, delta, _) = &frames[1];
    assert_eq!(delta["usage"], delta_usage);
    let (_, stop, _) = &frames[2];
    assert_eq!(stop["usage"], stop_usage);
}

fn tool_use_start(id: Json, name: Json) -> Json {
    obj(vec![
        ("type", str_val("tool_use")),
        ("id", id),
        ("name", name),
        ("input", obj(vec![])),
    ])
}

fn input_json_delta(partial_json: &str) -> Vec<Json> {
    vec![obj(vec![
        ("type", str_val("input_json_delta")),
        ("partial_json", str_val(partial_json)),
    ])]
}

fn thinking_start(thinking: &str, signature: &str) -> Json {
    obj(vec![
        ("type", str_val("thinking")),
        ("thinking", str_val(thinking)),
        ("signature", str_val(signature)),
    ])
}

fn content_block_variant_blocks() -> [Json; 8] {
    [
        obj(vec![("type", str_val("text"))]),
        obj(vec![("type", str_val("text")), ("text", Json::Null)]),
        obj(vec![
            ("type", str_val("thinking")),
            ("thinking", str_val("Thought")),
            ("signature", str_val("sig")),
        ]),
        thinking_start("", ""),
        obj(vec![
            ("type", str_val("redacted_thinking")),
            ("data", str_val("cipher")),
        ]),
        obj(vec![
            ("type", str_val("tool_use")),
            ("id", str_val("tool_1")),
            ("name", str_val("lookup")),
            ("input", obj(vec![("x", num(1))])),
        ]),
        obj(vec![("type", str_val("tool_use")), ("input", Json::Null)]),
        obj(vec![("type", str_val("tool_use"))]),
    ]
}

#[rstest]
#[case::text_absent(
    obj(vec![("type", str_val("text"))]),
    obj(vec![("type", str_val("text")), ("text", str_val(""))]),
    vec![obj(vec![("type", str_val("text_delta")), ("text", str_val(""))])]
)]
#[case::text_null(
    obj(vec![("type", str_val("text")), ("text", Json::Null)]),
    obj(vec![("type", str_val("text")), ("text", str_val(""))]),
    vec![obj(vec![("type", str_val("text_delta")), ("text", Json::Null)])]
)]
#[case::thinking_with_signature(
    obj(vec![
        ("type", str_val("thinking")),
        ("thinking", str_val("Thought")),
        ("signature", str_val("sig")),
    ]),
    thinking_start("", ""),
    vec![
        obj(vec![("type", str_val("thinking_delta")), ("thinking", str_val("Thought"))]),
        obj(vec![("type", str_val("signature_delta")), ("signature", str_val("sig"))]),
    ]
)]
#[case::thinking_empty(thinking_start("", ""), thinking_start("", ""), vec![])]
#[case::redacted_thinking_drops_data(
    obj(vec![("type", str_val("redacted_thinking")), ("data", str_val("cipher"))]),
    obj(vec![("type", str_val("redacted_thinking"))]),
    vec![]
)]
#[case::tool_use_with_input(
    obj(vec![
        ("type", str_val("tool_use")),
        ("id", str_val("tool_1")),
        ("name", str_val("lookup")),
        ("input", obj(vec![("x", num(1))])),
    ]),
    tool_use_start(str_val("tool_1"), str_val("lookup")),
    input_json_delta("{\"x\": 1}")
)]
#[case::tool_use_input_null(
    obj(vec![("type", str_val("tool_use")), ("input", Json::Null)]),
    tool_use_start(Json::Null, Json::Null),
    input_json_delta("null")
)]
#[case::tool_use_input_absent(
    obj(vec![("type", str_val("tool_use"))]),
    tool_use_start(Json::Null, Json::Null),
    input_json_delta("{}")
)]
fn content_block_variants(
    #[case] block: Json,
    #[case] expected_start: Json,
    #[case] expected_deltas: Vec<Json>,
) {
    let response = obj(vec![("content", arr(vec![block]))]);
    let frames = capture(&response).expect("content block fixture builds frames");
    assert_eq!(
        find_block(&frames, "content_block_start", 0)["content_block"],
        expected_start
    );
    assert_eq!(deltas_at(&frames, 0), expected_deltas);
    assert_eq!(
        frames
            .iter()
            .filter(|(name, _, _)| name == "content_block_stop")
            .count(),
        1
    );
}

#[test]
fn content_block_variants_index_by_position() {
    let response = obj(vec![(
        "content",
        arr(content_block_variant_blocks().to_vec()),
    )]);
    let frames = capture(&response).expect("content block fixture builds frames");
    let stop_indexes: Vec<i64> = frames
        .iter()
        .filter(|(name, _, _)| name == "content_block_stop")
        .map(|(_, data, _)| data["index"].as_i64().expect("index is a number"))
        .collect();
    assert_eq!(stop_indexes, vec![0, 1, 2, 3, 4, 5, 6, 7]);
    assert_eq!(
        find_block(&frames, "content_block_start", 0)["content_block"]["type"],
        str_val("text")
    );
    assert_eq!(
        find_block(&frames, "content_block_start", 7)["content_block"]["type"],
        str_val("tool_use")
    );
}

#[test]
fn passthrough_and_invalid_content() {
    let response = obj(vec![(
        "content",
        arr(vec![
            obj(vec![("type", str_val("future")), ("x", num(1))]),
            obj(vec![("x", num(1))]),
            obj(vec![("type", str_val("text")), ("text", str_val("\u{e9}"))]),
        ]),
    )]);
    let frames = capture(&response).expect("passthrough content builds frames");

    assert_eq!(
        find_block(&frames, "content_block_start", 0)["content_block"],
        obj(vec![("type", str_val("future")), ("x", num(1))])
    );
    assert_eq!(deltas_at(&frames, 0), Vec::<Json>::new());

    assert_eq!(
        find_block(&frames, "content_block_start", 1)["content_block"],
        obj(vec![("x", num(1))])
    );
    assert_eq!(deltas_at(&frames, 1), Vec::<Json>::new());

    let (_, _, text_delta_bytes) = frames
        .iter()
        .find(|(name, data, _)| {
            name == "content_block_delta" && data.get("index") == Some(&Json::from(2_i64))
        })
        .expect("text delta frame exists");
    let text_delta_text = String::from_utf8(text_delta_bytes.clone()).expect("frame is ascii");
    assert!(text_delta_text.contains("\\u00e9"));
    assert!(!text_delta_text.contains('\u{e9}'));

    assert_eq!(
        MessagesFakeStream::new(&obj(vec![("content", Json::Null)])).err(),
        Some(MessagesFakeStreamError::ContentNotIterable)
    );
    assert_eq!(
        MessagesFakeStream::new(&obj(vec![])).map(Iterator::count),
        Ok(3)
    );
    assert_eq!(
        MessagesFakeStream::new(&obj(vec![("content", arr(vec![Json::Null]))])).err(),
        Some(MessagesFakeStreamError::ContentBlockNotMapping)
    );
}

#[test]
fn shared_cursor() {
    let response = obj(vec![(
        "content",
        arr(vec![obj(vec![
            ("type", str_val("text")),
            ("text", str_val("A")),
        ])]),
    )]);
    let mut stream = MessagesFakeStream::new(&response).expect("single text block builds frames");

    let (first, _) = parse_frame(&stream.next().expect("message_start is first"));
    assert_eq!(first, "message_start");
    let (second, _) = parse_frame(&stream.next().expect("content_block_start is second"));
    assert_eq!(second, "content_block_start");
    let (third, _) = parse_frame(&stream.next().expect("content_block_delta is third"));
    assert_eq!(third, "content_block_delta");
    let (fourth, _) = parse_frame(&stream.next().expect("content_block_stop is fourth"));
    assert_eq!(fourth, "content_block_stop");
    let (fifth, _) = parse_frame(&stream.next().expect("message_delta is fifth"));
    assert_eq!(fifth, "message_delta");
    let (sixth, _) = parse_frame(&stream.next().expect("message_stop is sixth"));
    assert_eq!(sixth, "message_stop");

    assert_eq!(stream.next(), None);
    assert_eq!(stream.next(), None);
}
