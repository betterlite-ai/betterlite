use std::collections::BTreeMap;
use std::time::Duration;

use secrecy::SecretString;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretTimeout {
    Total(Duration),
    Phases {
        connect: Option<Duration>,
        read: Option<Duration>,
        write: Option<Duration>,
        pool: Option<Duration>,
    },
}

#[derive(Debug)]
pub struct SecretOptions<'borrowed, P> {
    pub optional_params: Option<&'borrowed P>,
    pub timeout: Option<SecretTimeout>,
}

impl<P> Copy for SecretOptions<'_, P> {}
impl<P> Clone for SecretOptions<'_, P> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<P> Default for SecretOptions<'_, P> {
    fn default() -> Self {
        Self {
            optional_params: None,
            timeout: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SecretTags {
    Mapping(BTreeMap<String, String>),
    List(Vec<SecretTag>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct SecretTag {
    pub key: String,
    pub value: String,
}

#[derive(Debug)]
pub struct SecretWrite<'borrowed, P> {
    pub secret_name: &'borrowed str,
    pub secret_value: &'borrowed SecretString,
    pub description: Option<&'borrowed str>,
    pub options: SecretOptions<'borrowed, P>,
    pub tags: Option<&'borrowed SecretTags>,
}

#[derive(Debug)]
pub struct SecretRotation<'borrowed, P> {
    pub current_secret_name: &'borrowed str,
    pub new_secret_name: &'borrowed str,
    pub new_secret_value: &'borrowed SecretString,
    pub options: SecretOptions<'borrowed, P>,
}

#[derive(Debug)]
pub struct DeleteOptions<'borrowed, P> {
    pub recovery_window_in_days: u32,
    pub operation: SecretOptions<'borrowed, P>,
}

impl<P> Default for DeleteOptions<'_, P> {
    fn default() -> Self {
        Self {
            recovery_window_in_days: 7,
            operation: SecretOptions::default(),
        }
    }
}
