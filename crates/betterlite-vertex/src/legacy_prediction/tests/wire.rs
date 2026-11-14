use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;

use crate::legacy_prediction::error::PredictionError;
use crate::legacy_prediction::frames::PredictionFrames;
use crate::legacy_prediction::records::{
    ChunkKind, LegacyChunk, chat_reply, decode_legacy_prediction,
};
use crate::legacy_prediction::tensor::{Tensor, chunk_of};

const FRAME_LIMIT: usize = 4_194_304;

fn number(value: i64) -> Json {
    Json::Number(Number::from(value))
}

fn float(value: f64) -> Json {
    Json::Number(Number::from_f64(value).expect("a finite number"))
}

fn tensor(text: &str) -> Tensor {
    serde_json::from_str(text).expect("a tensor")
}

fn encoded(value: &Json) -> String {
    serde_json::to_string(&Tensor::of_json(value).expect("a tensor")).expect("tensor JSON")
}

#[test]
fn legacy_prediction_tensor_encodes_values() {
    assert_eq!(
        Tensor::of_json(&Json::Null).expect("null"),
        Tensor::default()
    );
    assert_eq!(encoded(&Json::Bool(true)), r#"{"intVal":[1]}"#);
    assert_eq!(encoded(&Json::Bool(false)), r#"{"intVal":[0]}"#);
    assert_eq!(encoded(&number(7)), r#"{"intVal":[7]}"#);
    assert_eq!(encoded(&float(1.5)), r#"{"floatVal":[1.5]}"#);
    assert_eq!(
        encoded(&Json::Text(String::from("a"))),
        r#"{"stringVal":["a"]}"#
    );
    assert_eq!(
        encoded(&Json::List(vec![Json::Text(String::from("a")), number(1)])),
        r#"{"listVal":[{"stringVal":["a"]},{"intVal":[1]}]}"#
    );
    assert_eq!(
        encoded(&Json::Object(IndexMap::from([(
            String::from("k"),
            number(1)
        )]))),
        r#"{"structVal":{"k":{"intVal":[1]}}}"#
    );
    assert!(matches!(
        Tensor::of_json(&number(5_000_000_000)),
        Err(PredictionError::WrongFieldType { field: "intVal" })
    ));
}

#[test]
fn legacy_prediction_tensor_decodes_each_field() {
    let cases = [
        (r#"{"boolVal":[true]}"#, Json::Bool(true)),
        (
            r#"{"boolVal":[true,false]}"#,
            Json::List(vec![Json::Bool(true), Json::Bool(false)]),
        ),
        (r#"{"floatVal":[1.5]}"#, float(1.5)),
        (r#"{"doubleVal":[2.5]}"#, float(2.5)),
        (r#"{"intVal":[3]}"#, number(3)),
        (r#"{"int64Val":["4"]}"#, number(4)),
        (r#"{"uintVal":[5]}"#, number(5)),
        (r#"{"uint64Val":["6"]}"#, number(6)),
        (r#"{"stringVal":["s"]}"#, Json::Text(String::from("s"))),
        (r#"{"bytesVal":["b"]}"#, Json::Text(String::from("b"))),
        (
            r#"{"listVal":[{"intVal":[1]},{"intVal":[2]}]}"#,
            Json::List(vec![number(1), number(2)]),
        ),
        (
            r#"{"structVal":{"k":{"intVal":[1]}}}"#,
            Json::Object(IndexMap::from([(String::from("k"), number(1))])),
        ),
        (r#"{"tensorVal":"t"}"#, Json::Text(String::from("t"))),
        ("{}", Json::Null),
    ];
    for (text, expected) in cases {
        assert_eq!(tensor(text).to_json().expect(text), expected, "{text}");
    }
    assert!(matches!(
        tensor(r#"{"int64Val":["x"]}"#).to_json(),
        Err(PredictionError::WrongFieldType { field: "int64Val" })
    ));
    assert!(matches!(
        tensor(r#"{"uint64Val":["-1"]}"#).to_json(),
        Err(PredictionError::WrongFieldType { field: "uint64Val" })
    ));
}

#[test]
fn legacy_prediction_chunk_needs_one_output() {
    assert!(matches!(
        chunk_of(ChunkKind::Text, b"{}"),
        Err(PredictionError::MultipleOutputs { count: 0 })
    ));
    assert!(matches!(
        chunk_of(ChunkKind::Text, br#"{"outputs":[{},{}]}"#),
        Err(PredictionError::MultipleOutputs { count: 2 })
    ));
    assert!(matches!(
        chunk_of(ChunkKind::Text, b"nope"),
        Err(PredictionError::Decode { .. })
    ));
}

#[test]
fn legacy_prediction_chunk_reads_chat_candidates() {
    let object = br#"{"outputs":[{"structVal":{"candidates":{"listVal":[{"structVal":{"content":{"stringVal":["hi"]}}}]},"safetyAttributes":{"listVal":[{"structVal":{}}]}}}]}"#;
    assert_eq!(
        chunk_of(ChunkKind::Chat, object).expect("a chat chunk"),
        LegacyChunk {
            text: String::from("hi")
        }
    );
}

#[test]
fn legacy_prediction_frames_track_strings_and_nesting() {
    let mut frames = PredictionFrames::default();
    let ready = frames
        .push(b"[ {\"a\": \"x }\\\" {\", \"b\": [1, 2]} , {\"c\": 1}, {\"d\": \"\\\\\"}\n]")
        .expect("frames");
    assert_eq!(
        ready,
        vec![
            br#"{"a":"x }\" {","b":[1,2]}"#.to_vec(),
            br#"{"c":1}"#.to_vec(),
            br#"{"d":"\\"}"#.to_vec(),
        ]
    );
    assert!(frames.finish().is_ok());
    assert!(frames.push(b" \n\t").expect("trailing space").is_empty());
    assert!(matches!(
        frames.push(b"x"),
        Err(PredictionError::StreamShape)
    ));
}

#[test]
fn legacy_prediction_frames_limit_one_object() {
    let mut frames = PredictionFrames::default();
    assert!(frames.push(b"[{").expect("the open").is_empty());
    let filler = vec![b'x'; FRAME_LIMIT - 1];
    assert!(
        frames
            .push(&filler)
            .expect("a frame at the limit")
            .is_empty()
    );
    assert!(matches!(
        frames.push(b"x"),
        Err(PredictionError::TooLarge { limit: FRAME_LIMIT })
    ));
}

#[test]
fn legacy_prediction_body_limit_is_four_mebibytes() {
    let mut body = br#"{"predictions":[]}"#.to_vec();
    body.resize(FRAME_LIMIT, b' ');
    decode_legacy_prediction(&body).expect("a body at the limit");
    body.push(b' ');
    assert!(matches!(
        decode_legacy_prediction(&body),
        Err(PredictionError::TooLarge { limit: FRAME_LIMIT })
    ));
}

#[test]
fn legacy_prediction_chat_safety_counts() {
    let two_candidates = |safety: &str| {
        let body = format!(
            r#"{{"predictions":[{{"candidates":[{{"content":"a"}},{{"content":"b"}}],"safetyAttributes":{safety}}}]}}"#
        );
        decode_legacy_prediction(body.as_bytes()).expect("a body")
    };
    assert!(matches!(
        chat_reply(&two_candidates("[{}]")),
        Err(PredictionError::WrongFieldType {
            field: "safetyAttributes"
        })
    ));
    assert!(chat_reply(&two_candidates("[{},{}]")).is_ok());
    assert!(chat_reply(&two_candidates("[{},{},{}]")).is_ok());
}
