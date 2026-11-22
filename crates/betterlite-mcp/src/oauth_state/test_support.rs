#![cfg(test)]

use super::SealCipher;

pub(crate) struct PlainCipher;

impl SealCipher for PlainCipher {
    fn encrypt(&self, plain: &str) -> String {
        plain.to_owned()
    }

    fn decrypt(&self, value: &str, _field: &str) -> Option<String> {
        Some(value.to_owned())
    }
}

pub(crate) struct RefusingCipher;

impl SealCipher for RefusingCipher {
    fn encrypt(&self, plain: &str) -> String {
        plain.to_owned()
    }

    fn decrypt(&self, _value: &str, _field: &str) -> Option<String> {
        None
    }
}

pub(crate) struct FieldBoundCipher {
    pub(crate) field: &'static str,
}

impl SealCipher for FieldBoundCipher {
    fn encrypt(&self, plain: &str) -> String {
        plain.to_owned()
    }

    fn decrypt(&self, value: &str, field: &str) -> Option<String> {
        (field == self.field).then_some(value.to_owned())
    }
}
