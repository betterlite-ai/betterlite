#![no_main]

use std::str;

use betterlite_core::routes::{
    catalog::{CATALOG, RouteCatalog},
    pattern::parse_route,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if let (Ok(route), Ok(catalog)) = (str::from_utf8(bytes), RouteCatalog::parse(CATALOG)) {
        drop(parse_route(route));
        drop(catalog.call_types_for(route));
    }
});
