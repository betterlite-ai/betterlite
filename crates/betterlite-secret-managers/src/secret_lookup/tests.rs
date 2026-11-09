#![cfg(test)]

mod support;

use std::collections::BTreeMap;
use std::fs;
use std::sync::Arc;

use rstest::rstest;
use secrecy::{ExposeSecret, SecretString};

use crate::oidc::http_timeouts::OidcHttpTimeouts;
use crate::secret_lookup::SecretValue;
use crate::secret_read_dispatch::settings::SecretManagerSettings;
use crate::secret_types::{KeyManagementSystem, SecretAccessMode};

use self::support::{
    AzureReply, FakeAzureTokens, FakeManager, ManagerReply, ManualClock, Reply, aws_secrets_client,
    base_lookup, bool_value, capture_logs, custom_client, custom_client_with, environment, jwt,
    manager_state, scripted_lookup, text_value,
};

include!("tests/prefix.rs");
include!("tests/oidc.rs");
include!("tests/manager.rs");
include!("tests/env.rs");
include!("tests/typed.rs");
include!("tests/github_response.rs");
