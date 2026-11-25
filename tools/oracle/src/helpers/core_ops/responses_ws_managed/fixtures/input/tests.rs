#![cfg(test)]

use super::input::Input;
use betterlite_models::decisions::Json;
use rstest::rstest;

enum Boundary {
    Count(usize),
    Capture(usize),
    Frame(usize),
    ChunkDump(usize),
    ChunkJson(usize),
    Deadline(u64),
}

#[rstest]
#[case::at_count(Boundary::Count(4), true)]
#[case::over_count(Boundary::Count(3), false)]
#[case::at_capture(Boundary::Capture(0), true)]
#[case::over_capture(Boundary::Capture(1), false)]
#[case::at_frame(Boundary::Frame(512), true)]
#[case::over_frame(Boundary::Frame(511), false)]
#[case::at_chunk_dump(Boundary::ChunkDump(512), true)]
#[case::over_chunk_dump(Boundary::ChunkDump(511), false)]
#[case::at_chunk_json(Boundary::ChunkJson(512), true)]
#[case::over_chunk_json(Boundary::ChunkJson(511), false)]
#[case::zero_deadline(Boundary::Deadline(0), false)]
#[case::at_deadline(Boundary::Deadline(120_000), true)]
#[case::over_deadline(Boundary::Deadline(120_001), false)]
fn managed_inputs_are_bounded(#[case] boundary: Boundary, #[case] accepted: bool) {
    let mut input: Input = serde_json::from_str(
        r#"{
        "setup":{"model":"m","kwargs":{},"first_message":"first"},
        "frames":["second"],"disconnect":"closed",
        "scripts":[{"chunks":[{"kind":"object","dump":"payload"}],"raise_message":""}],
        "quota":null,"sink":{"message":"send"}
    }"#,
    )
    .unwrap();
    let (count, frame, capture) = match boundary {
        Boundary::Count(cap) => (cap, 4096, 4096),
        Boundary::Capture(reduction) => (
            1024,
            4096,
            serde_json::to_vec(&input)
                .unwrap()
                .len()
                .checked_sub(reduction)
                .unwrap(),
        ),
        Boundary::Frame(cap) => {
            input.frames.push("x".repeat(512));
            (1024, cap, 4096)
        }
        Boundary::ChunkDump(cap) => {
            input.scripts[0].chunks[0].dump = Some("x".repeat(512));
            (1024, cap, 4096)
        }
        Boundary::ChunkJson(cap) => {
            input.scripts[0].chunks[0].dump = None;
            input.scripts[0].chunks[0].value = Some(Json::Text("x".repeat(510)));
            (1024, cap, 4096)
        }
        Boundary::Deadline(millis) => {
            input.deadline_millis = millis;
            (1024, 4096, 4096)
        }
    };
    assert_eq!(input.check_bounds(count, frame, capture).is_ok(), accepted);
}
