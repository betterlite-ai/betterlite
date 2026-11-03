#![no_main]
use betterlite_auth::route_match::templates::{RouteValue, route_matches_pattern};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|source: &str| {
    if let Some((pattern, route)) = source.split_once('\0') {
        let _matched = route_matches_pattern(&RouteValue::Text(route), pattern);
    }
});
