#![no_main]

use std::collections::BTreeMap;
use std::str;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use libfuzzer_sys::fuzz_target;
use url::Url;

fuzz_target!(|bytes: &[u8]| {
    let Ok(text) = str::from_utf8(bytes.get(..4096).unwrap_or(bytes)) else {
        return;
    };
    let (proxy, no_proxy) = text.split_once('\n').unwrap_or((text, ""));
    let mut environment = BTreeMap::new();
    environment.insert("HTTPS_PROXY".to_owned(), proxy.to_owned());
    environment.insert("NO_PROXY".to_owned(), no_proxy.to_owned());
    let Ok(policy) = ProxyPolicy::resolve(&ProxyTrust::default(), &environment) else {
        return;
    };
    if let Ok(url) = Url::parse("https://api.example.com/v1") {
        let _route = policy.route(&url);
    }
});
