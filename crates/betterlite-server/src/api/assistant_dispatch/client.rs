use std::sync::Arc;
use std::time::Duration;

use betterlite_core::api::assistant_read_delete::request::{SuppliedClient, SuppliedClientKind};
use betterlite_core::api::assistant_records::ClientClass;
use betterlite_core::api::assistant_run_sdk::call::{RunClient, RunClientClass};
use betterlite_core::api::assistant_threads::SuppliedClient as ThreadClient;
use betterlite_openai::assistants::client::{ClientKind, SdkClient};
use secrecy::SecretString;

// The SDK class the caller built, as the proxy would have bound the client argument (main.py's isinstance checks).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SuppliedClass {
    OpenAi,
    AzureOpenAi,
    AsyncOpenAi,
    AsyncAzureOpenAi,
}

// What the three SDK owners call the same supplied class.
pub(super) struct ClassKinds {
    pub(super) client: ClientClass,
    pub(super) read_delete: SuppliedClientKind,
    pub(super) run: RunClientClass,
}

impl SuppliedClass {
    // the one place the class is translated, for the create, read/delete and run SDK owners alike
    #[must_use]
    pub(super) const fn kinds(self) -> ClassKinds {
        match self {
            Self::OpenAi => ClassKinds {
                client: ClientClass::OpenAi,
                read_delete: SuppliedClientKind::OpenAi,
                run: RunClientClass::OpenAi,
            },
            Self::AzureOpenAi => ClassKinds {
                client: ClientClass::AzureOpenAi,
                read_delete: SuppliedClientKind::AzureOpenAi,
                run: RunClientClass::Azure,
            },
            Self::AsyncOpenAi => ClassKinds {
                client: ClientClass::AsyncOpenAi,
                read_delete: SuppliedClientKind::AsyncOpenAi,
                run: RunClientClass::AsyncOpenAi,
            },
            Self::AsyncAzureOpenAi => ClassKinds {
                client: ClientClass::AsyncAzureOpenAi,
                read_delete: SuppliedClientKind::AsyncAzureOpenAi,
                run: RunClientClass::AsyncAzure,
            },
        }
    }
}

// The constructor arguments of a supplied SDK client: the identity the SDK owner must use unchanged.
#[derive(Debug, Clone)]
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
    // get_assistants and delete_assistant read the class to drop a sync client on azure, then reuse it
    #[must_use]
    pub fn read_delete(&self) -> SuppliedClient {
        SuppliedClient {
            kind: self.class.kinds().read_delete,
            api_base: self.api_base.clone(),
            api_key: self.api_key.clone(),
            api_version: self.api_version.clone(),
        }
    }

    // create_thread and get_thread take the client object itself, so its own base and key win over the facade
    #[must_use]
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
            class: self.class.kinds().run,
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
