#![no_main]

use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::tls::TlsPolicy;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let yaml = String::from_utf8_lossy(bytes.get(..4096).unwrap_or(bytes));
    let Ok(config) = (ConfigLoader {
        lookup: BTreeMap::new(),
    })
    .load(&yaml) else {
        return;
    };
    let mut environment = BTreeMap::new();
    environment.insert("SSL_CERT_FILE".to_owned(), "/missing/ca.pem".to_owned());
    let _policy = TlsPolicy::resolve(&config, &environment, None);
});
