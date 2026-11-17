#![no_main]

use betterlite_models::catalog::ModelCatalog;
use betterlite_vertex::gemini_live_client::state::LiveSessionState;
use betterlite_vertex::gemini_live_frames::decode::decode_gemini_live_frame_bytes;
use betterlite_vertex::gemini_live_frames::state::CallerState;
use betterlite_vertex::gemini_live_frames::transform::{
    ResponseContext, transform_realtime_response,
};
use libfuzzer_sys::fuzz_target;
use uuid::Uuid;

const MAX_FRAMES: usize = 8;
// every identifier repeats the draw count in four 32-bit groups, the way the Python helper's counted uuid4 does
const UUID_PATTERN: u128 = 0x0000_0001_0000_0001_0000_0001_0000_0001;

fuzz_target!(|bytes: &[u8]| {
    let catalog = ModelCatalog::default();
    let context = ResponseContext {
        model: "gemini-2.5-flash",
        logging_session_id: "fuzz-session",
        catalog: &catalog,
    };
    let mut session = LiveSessionState::default();
    let mut caller = CallerState::default();
    let mut drawn = 0_u128;
    // a newline starts the next frame, so one input carries a whole connection's turns
    for piece in bytes.split(|byte| *byte == b'\n').take(MAX_FRAMES) {
        let Ok(mut frame) = decode_gemini_live_frame_bytes(piece, usize::MAX) else {
            continue;
        };
        let mut uuids = || {
            drawn = drawn.saturating_add(1);
            Uuid::from_u128(drawn.saturating_mul(UUID_PATTERN))
        };
        let answer =
            transform_realtime_response(&mut frame, &caller, &mut session, &context, &mut uuids);
        if let Ok(next) = answer {
            caller = next.state;
        }
    }
});
