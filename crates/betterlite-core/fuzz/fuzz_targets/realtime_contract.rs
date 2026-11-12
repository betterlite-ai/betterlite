#![no_main]

use betterlite_core::api::realtime_contracts::decode::{
    RealtimeContractKind, RealtimeDecodeLimits, decode_realtime_contract, decode_realtime_frame,
};
use libfuzzer_sys::fuzz_target;

const LIMITS: RealtimeDecodeLimits = RealtimeDecodeLimits { max_bytes: 1 << 20 };

fuzz_target!(|bytes: &[u8]| {
    let Some((&kind, rest)) = bytes.split_first() else {
        return;
    };
    match RealtimeContractKind::try_from(kind) {
        Ok(contract) => drop(decode_realtime_contract(rest, contract, LIMITS)),
        Err(_) => drop(decode_realtime_frame(rest, LIMITS)),
    }
});
