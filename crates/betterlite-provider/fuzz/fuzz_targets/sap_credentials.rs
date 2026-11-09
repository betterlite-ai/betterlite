#![no_main]

use std::io;
use std::path::{Path, PathBuf};
use std::str::from_utf8;

use betterlite_provider::sap_auth::credentials::fetch_credentials;
use betterlite_provider::sap_auth::environment::SapEnvironment;
use libfuzzer_sys::fuzz_target;

struct FuzzEnvironment {
    text: String,
    config: bool,
}

impl SapEnvironment for FuzzEnvironment {
    fn var(&self, name: &str) -> Option<String> {
        match (self.config, name) {
            (false, "AICORE_SERVICE_KEY" | "VCAP_SERVICES") => Some(self.text.clone()),
            (true, "AICORE_CONFIG") => Some("/fuzz/config.json".to_owned()),
            _ => None,
        }
    }

    fn file_exists(&self, path: &Path) -> bool {
        self.config && path == Path::new("/fuzz/config.json")
    }

    fn read_file(&self, path: &Path) -> Result<String, io::Error> {
        if self.config && path == Path::new("/fuzz/config.json") {
            return Ok(self.text.clone());
        }
        Err(io::Error::from(io::ErrorKind::NotFound))
    }

    fn home_dir(&self) -> PathBuf {
        PathBuf::from("/fuzz")
    }
}

fuzz_target!(|bytes: &[u8]| {
    let Ok(text) = from_utf8(bytes) else {
        return;
    };
    let service_key = FuzzEnvironment {
        text: text.to_owned(),
        config: false,
    };
    let _decoded = fetch_credentials(&service_key, None, None);
    let configured = FuzzEnvironment {
        text: text.to_owned(),
        config: true,
    };
    let _configured = fetch_credentials(&configured, None, None);
});
