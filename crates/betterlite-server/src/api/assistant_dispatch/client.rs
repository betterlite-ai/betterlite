use std::sync::Arc;
use std::time::Duration;

use betterlite_azure::azure_assistant_definitions::client::AzureAssistantsClient;
use betterlite_azure::azure_assistant_definitions::error::DefinitionsError;
use betterlite_azure::azure_urls::EndpointSelection;
use betterlite_azure::endpoints::projection::{AzureClientInputs, HttpClient};
use betterlite_config::loader::EnvironmentLookup;
use betterlite_core::api::assistant_read_delete::request::{SuppliedClient, SuppliedClientKind};
use betterlite_core::api::assistant_records::ClientClass;
use betterlite_core::api::assistant_run_sdk::call::{RunClient, RunClientClass};
use betterlite_core::api::assistant_threads::SuppliedClient as ThreadClient;
use betterlite_http::client_kind::ClientKind;
use betterlite_openai::assistants::client::SdkClient;
use betterlite_openai::assistants::transport::AssistantsTransport;
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
    pub fn thread<T>(
        &self,
        transport: Arc<T>,
        environment: &dyn EnvironmentLookup,
        default_api_version: &str,
    ) -> Result<ThreadClient<'_, T>, DefinitionsError>
    where
        T: AssistantsTransport,
    {
        let kind = match self.class {
            SuppliedClass::OpenAi | SuppliedClass::AzureOpenAi => ClientKind::Sync,
            SuppliedClass::AsyncOpenAi | SuppliedClass::AsyncAzureOpenAi => {
                ClientKind::Asynchronous
            }
        };
        match self.class {
            SuppliedClass::OpenAi | SuppliedClass::AsyncOpenAi => {
                Ok(ThreadClient::Openai(SdkClient {
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
                }))
            }
            SuppliedClass::AzureOpenAi | SuppliedClass::AsyncAzureOpenAi => {
                let inputs = self.azure_inputs(kind, environment, default_api_version);
                AzureAssistantsClient::new(kind, inputs, environment, transport)
                    .map(ThreadClient::Azure)
            }
        }
    }

    fn azure_inputs(
        &self,
        kind: ClientKind,
        environment: &dyn EnvironmentLookup,
        default_api_version: &str,
    ) -> AzureClientInputs<'static> {
        AzureClientInputs {
            endpoint: EndpointSelection {
                azure_endpoint: self.api_base.clone(),
                base_url: None,
            },
            api_version: self
                .api_version
                .clone()
                .or_else(|| environment.lookup("AZURE_API_VERSION"))
                .unwrap_or_else(|| default_api_version.to_owned()),
            api_key: self.api_key.clone(),
            azure_ad_token: None,
            token_provider: None,
            max_retries: 0,
            timeout: self.timeout,
            http_client: HttpClient {
                is_async: kind == ClientKind::Asynchronous,
                shared: false,
            },
        }
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
