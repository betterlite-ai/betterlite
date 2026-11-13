#![no_main]
use betterlite_server::auth::route_match::decoded_route_path;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|path: &str| {
    if let Some(decoded) = decoded_route_path(path) {
        assert!(path.len() <= 0x2000);
        assert!(decoded.len() <= path.len());
    }
});
