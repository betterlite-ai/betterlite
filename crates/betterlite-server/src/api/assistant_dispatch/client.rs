use std::sync::Arc;
use std::time::Duration;

use betterlite_core::api::assistant_read_delete::request::{SuppliedClient, SuppliedClientKind};
use betterlite_core::api::assistant_records::ClientClass;
use betterlite_core::api::assistant_run_sdk::call::{RunClient, RunClientClass};
use betterlite_core::api::assistant_threads::SuppliedClient as ThreadClient;
use betterlite_openai::assistants::client::{ClientKind, SdkClient};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

// The SDK class the caller built, as the proxy would have bound the client argument (main.py's isinstance checks).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SuppliedClass {
    OpenAi,
    AzureOpenAi,
    AsyncOpenAi,
    AsyncAzureOpenAi,
}

// The constructor arguments of a supplied SDK client: the identity the SDK owner must use unchanged.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantClient {
    pub class: SuppliedClass,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub api_version: Option<String>,
    pub timeout: Option<f64>,
    pub max_retries: Option<i64>,
    pub organization: Option<String>,
}

impl AssistantClient {
    #[must_use]
    pub const fn create_class(&self) -> ClientClass {
        match self.class {
            SuppliedClass::OpenAi => ClientClass::OpenAi,
            SuppliedClass::AzureOpenAi => ClientClass::AzureOpenAi,
            SuppliedClass::AsyncOpenAi => ClientClass::AsyncOpenAi,
            SuppliedClass::AsyncAzureOpenAi => ClientClass::AsyncAzureOpenAi,
        }
    }

    // get_assistants and delete_assistant read the class to drop a sync client on azure, then reuse it
    #[must_use]
    pub fn read_delete(&self) -> SuppliedClient {
        SuppliedClient {
            kind: match self.class {
                SuppliedClass::OpenAi => SuppliedClientKind::OpenAi,
                SuppliedClass::AzureOpenAi => SuppliedClientKind::AzureOpenAi,
                SuppliedClass::AsyncOpenAi => SuppliedClientKind::AsyncOpenAi,
                SuppliedClass::AsyncAzureOpenAi => SuppliedClientKind::AsyncAzureOpenAi,
            },
            api_base: self.api_base.clone(),
            api_key: self.api_key.clone(),
            api_version: self.api_version.clone(),
        }
    }

    // create_thread and get_thread take the client object itself, so its own base and key win over the facade
    pub fn thread<T>(&self, transport: Arc<T>) -> ThreadClient<'_, T> {
        let kind = match self.class {
            SuppliedClass::OpenAi | SuppliedClass::AzureOpenAi => ClientKind::Sync,
            SuppliedClass::AsyncOpenAi | SuppliedClass::AsyncAzureOpenAi => ClientKind::Async,
        };
        ThreadClient::Openai(SdkClient {
            kind,
            api_key: self.api_key.clone(),
            base_url: self.api_base.clone(),
            timeout: self
                .timeout
                .and_then(|seconds| Duration::try_from_secs_f64(seconds).ok()),
            max_retries: self
                .max_retries
                .map(|retries| u32::try_from(retries.max(0)).unwrap_or(u32::MAX)),
            organization: self.organization.clone(),
            transport,
        })
    }

    // arun_thread reads the same constructor arguments through the supplied-client record
    #[must_use]
    pub fn run(&self) -> RunClient {
        RunClient {
            class: match self.class {
                SuppliedClass::OpenAi => RunClientClass::OpenAi,
                SuppliedClass::AzureOpenAi => RunClientClass::Azure,
                SuppliedClass::AsyncOpenAi => RunClientClass::AsyncOpenAi,
                SuppliedClass::AsyncAzureOpenAi => RunClientClass::AsyncAzure,
            },
            api_key: self.api_key.clone(),
            base_url: self.api_base.clone(),
            api_version: self.api_version.clone(),
            azure_ad_token: None,
            timeout: self.timeout,
            max_retries: self.max_retries,
            organization: self.organization.clone(),
        }
    }
}
