#![cfg(test)]

mod corpus;
mod mistral_files;

use betterlite_config::sections::mistral_resources::Settings;
use secrecy::SecretString;

pub(super) fn settings(api_base: Option<&str>, api_key: Option<&str>) -> Settings {
    Settings {
        api_base: api_base.map(str::to_owned),
        api_key: api_key.map(SecretString::from),
    }
}
