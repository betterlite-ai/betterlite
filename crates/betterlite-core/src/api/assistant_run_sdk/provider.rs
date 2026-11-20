use std::sync::Arc;
use std::time::Duration;

use betterlite_azure::assistants::error::AssistantsRunError;
use betterlite_azure::assistants::operations::AssistantsRunsApi;
use betterlite_azure::azure_assistant_definitions::client::AzureAssistantsClient;
use betterlite_azure::azure_assistant_definitions::operations::AssistantsApi as AzureAssistantsApi;
use betterlite_openai::assistants::client::SdkClient;
use betterlite_openai::assistants::operations::AssistantsApi as OpenAiAssistantsApi;
use betterlite_openai::assistants::transport::AssistantsTransport;
use betterlite_openai::thread_runs::operations::RunsApi;
use betterlite_openai::thread_runs::records::RunReply;
use betterlite_openai::thread_runs::stream::{RunStream, RunStreamTransport};

use super::service::{AzureRun, OpenAiRun, RunOperations, RunTarget};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(600);

pub struct ProviderRuns<T> {
    pub transport: Arc<T>,
}

impl<T> RunOperations for ProviderRuns<T>
where
    T: AssistantsTransport + RunStreamTransport,
{
    type Stream = RunStream<T>;

    async fn poll(&self, target: RunTarget<'_>) -> Result<Option<RunReply>, AssistantsRunError> {
        match target {
            RunTarget::OpenAi(run) => {
                let client = self.openai_client(&run);
                let reply = RunsApi {
                    transport: Arc::clone(&self.transport),
                }
                .create_and_poll(&client, &run.request)
                .await?;
                Ok(Some(reply))
            }
            RunTarget::Azure(run) => {
                let client = self.azure_client(&run)?;
                let reply = AssistantsRunsApi {
                    transport: Arc::clone(&self.transport),
                }
                .poll(&client, &run.request)
                .await?;
                Ok(Some(reply))
            }
        }
    }

    // both managers are built without a request: the provider call starts when the caller reads the stream
    fn stream(&self, target: RunTarget<'_>) -> Result<RunStream<T>, AssistantsRunError> {
        match target {
            RunTarget::OpenAi(run) => {
                let client = self.openai_client(&run);
                Ok(RunsApi {
                    transport: Arc::clone(&self.transport),
                }
                .create_stream(&client, &run.request, run.handler.clone())?)
            }
            RunTarget::Azure(run) => {
                let client = self.azure_client(&run)?;
                Ok(AssistantsRunsApi {
                    transport: Arc::clone(&self.transport),
                }
                .stream(&client, &run.request, run.handler.clone())?)
            }
        }
    }
}

impl<T> ProviderRuns<T>
where
    T: AssistantsTransport + RunStreamTransport,
{
    // get_openai_client(kind, options, client): a supplied client is returned untouched, whatever its class
    fn openai_client(&self, run: &OpenAiRun) -> SdkClient<T> {
        let supplied = run.supplied.as_ref().map(|client| SdkClient {
            kind: client.kind(),
            api_key: client.api_key.clone(),
            base_url: client.base_url.clone(),
            timeout: run.supplied_timeout,
            max_retries: client
                .max_retries
                .and_then(|value| u32::try_from(value).ok()),
            organization: client.organization.clone(),
            transport: Arc::clone(&self.transport),
        });
        OpenAiAssistantsApi {
            transport: Arc::clone(&self.transport),
        }
        .get_openai_client(run.kind, run.options.clone(), supplied)
    }

    fn azure_client<'data>(
        &self,
        run: &AzureRun<'data>,
    ) -> Result<AzureAssistantsClient<'data, T>, AssistantsRunError> {
        let api = AzureAssistantsApi {
            transport: Arc::clone(&self.transport),
        };
        // an AzureOpenAI instance keeps its own endpoint, api-version, key and token even under an openai provider
        let supplied = run.supplied.as_ref().map(|client| AzureAssistantsClient {
            kind: client.kind(),
            inputs: None,
            transport: Arc::clone(&self.transport),
            base_url: client.base_url.clone().unwrap_or_default(),
            api_version: client.api_version.clone().unwrap_or_default(),
            key: client.api_key.clone(),
            token: client.azure_ad_token.clone(),
            deadline: run.supplied_timeout.unwrap_or(DEFAULT_TIMEOUT),
        });
        api.azure_client(&run.client, supplied, run.kind)
            .map_err(AssistantsRunError::Client)
    }
}
