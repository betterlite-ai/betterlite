use secrecy::ExposeSecret as _;

use super::linked_rows::{KeyRow, LinkedKeys};

impl LinkedKeys for KeyRow {
    fn counter_key(&self) -> String {
        format!("spend:key:{}", self.token.expose_secret())
    }

    fn cache_keys(&self) -> Vec<String> {
        vec![self.token.expose_secret().to_owned()]
    }
}

mod tests;
