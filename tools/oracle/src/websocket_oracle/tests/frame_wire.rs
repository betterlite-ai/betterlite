#![cfg(test)]

use super::{Arguments, ServiceInput, ServiceTag, TranscriptObservation, fixtures, limits, valid};
use crate::helpers::{HelperCall, invoke};
use crate::websocket_driver::HandshakeObservation;
use crate::websocket_oracle::decode::decode_ws_case;

#[rstest::rstest]
#[case::text("text: setup")]
#[case::binary("binary: provider.bin")]
fn helper_roundtrip(#[case] payload: &str) {
    let yaml = valid().replace("text: setup", payload);
    let case = decode_ws_case(yaml.as_bytes(), &limits(), &fixtures()).unwrap();
    let defaults = Arguments::default();
    let input = ServiceInput {
        service: ServiceTag::RealtimeProbe,
        input: Arguments {
            frames: case.frames.clone(),
            ..defaults
        },
    };
    let encoded = serde_json::to_value(input.to_wire()).unwrap();
    let decoded: ServiceInput = serde_json::from_value(encoded.clone()).unwrap();
    assert_eq!(decoded.input.frames, case.frames);
    let output = invoke(&HelperCall {
        operation: "websocket_oracle".to_owned(),
        input: encoded,
    });
    assert_eq!(output.error.unwrap().class, "ServiceError");
    let observation = TranscriptObservation {
        handshake: HandshakeObservation {
            status: 101,
            negotiated_subprotocol: None,
        },
        frames: decoded.input.frames,
        close: None,
    };
    let wire = serde_json::to_vec(&observation).unwrap();
    let restored: TranscriptObservation = serde_json::from_slice(&wire).unwrap();
    assert_eq!(restored, observation);
}

#[rstest::rstest]
#[case::top_level(false)]
#[case::service_input(true)]
fn fixture_rejects_inline_bytes(#[case] nested: bool) {
    let frame = "direction: client_to_provider, action: send, text: x, binary_data: [1, 2, 3]";
    let yaml = if nested {
        format!("{}\ninput:\n  frames: [{{{frame}}}]\n", valid())
    } else {
        valid().replace("text: setup", "text: setup\n    binary_data: [1, 2, 3]")
    };
    decode_ws_case(yaml.as_bytes(), &limits(), &fixtures()).unwrap_err();
}
