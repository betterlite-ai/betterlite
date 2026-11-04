#![no_main]

use std::collections::BTreeMap;
use std::fs;
use std::process;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::tls::TlsPolicy;
use betterlite_http::tls::apply_policy;
use libfuzzer_sys::fuzz_target;
use reqwest::ClientBuilder;

const MAX_PEM_INPUT: usize = 4 * 1024 * 1024 + 1;

fuzz_target!(|bytes: &[u8]| {
    let pem = bytes.get(..MAX_PEM_INPUT).unwrap_or(bytes);
    let path = std::env::temp_dir().join(format!("betterlite-http-policy-{}.pem", process::id()));
    if fs::write(&path, pem).is_err() {
        return;
    }
    let Some(path_text) = path.to_str() else {
        let _ = fs::remove_file(path);
        return;
    };
    let Ok(config) = (ConfigLoader {
        lookup: BTreeMap::new(),
    })
    .load("") else {
        let _ = fs::remove_file(path);
        return;
    };
    let environment = BTreeMap::from([("SSL_CERT_FILE".to_owned(), path_text.to_owned())]);
    if let Ok(policy) = TlsPolicy::resolve(&config, &environment, None) {
        let _builder = apply_policy(ClientBuilder::new(), &policy);
    }
    let _ = fs::remove_file(path);
});
