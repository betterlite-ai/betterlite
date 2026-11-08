#![no_main]

use std::str;

use betterlite_config::sections::provider_url_destination::pattern::AllowedDestination;
use betterlite_http::url_policy::base::ConfiguredBase;
use betterlite_http::url_policy::destination::{candidates, destination_allowed};
use betterlite_http::url_policy::origin::assert_same_origin;
use libfuzzer_sys::fuzz_target;
use url::Url;

fuzz_target!(|bytes: &[u8]| {
    let Ok(text) = str::from_utf8(bytes.get(..4096).unwrap_or(bytes)) else {
        return;
    };
    let mut lines = text.lines();
    let candidate = lines.next().unwrap_or_default();
    let expected = lines.next().unwrap_or_default();
    let _origin = assert_same_origin(candidate, expected);
    let url = lines.next().unwrap_or_default();
    let allowed: Vec<AllowedDestination> = lines.filter_map(AllowedDestination::parse).collect();
    let _allowed = destination_allowed(url, &allowed);
    let _candidates = candidates(url);
    let _base = Url::parse(url)
        .ok()
        .and_then(|parsed| ConfiguredBase::parse(parsed, "api_base").ok());
});
