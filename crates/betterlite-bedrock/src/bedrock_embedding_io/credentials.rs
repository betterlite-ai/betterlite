use std::collections::BTreeMap;
use std::future::Future;
use std::thread::{self, JoinHandle};

use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_secret_managers::secret_lookup::SecretLookup;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::value::RawValue;
use tokio::sync::{mpsc, oneshot};

use super::error::EmbeddingIoError;
use crate::aws_endpoints::region::DEFAULT_AWS_REGION;
use crate::aws_settings::pop_aws_auth_params;
use crate::aws_settings::sources::Credentials;
use crate::credentials::error::CredentialError;
use crate::signing::target::auth_settings;

const REGION_NAME_VARIABLE: &str = "AWS_REGION_NAME";
const REGION_VARIABLE: &str = "AWS_REGION";
const REGION_WORKER_QUEUE_CAPACITY: usize = 16;

type RegionJob = Box<dyn FnOnce() + Send + 'static>;

// Keep the bounded reader service owned independently of request futures.
pub struct ManagedRegionWorker {
    sender: Option<mpsc::Sender<RegionJob>>,
    worker_thread: Option<JoinHandle<()>>,
}

impl ManagedRegionWorker {
    // Avoid entering Tokio on the thread that runs synchronous secret reads.
    pub fn new() -> Result<Self, EmbeddingIoError> {
        let (sender, mut receiver) = mpsc::channel::<RegionJob>(REGION_WORKER_QUEUE_CAPACITY);
        let worker_thread = thread::Builder::new()
            .name("bedrock-region-secret-reader".to_owned())
            .spawn(move || {
                while let Some(job) = receiver.blocking_recv() {
                    job();
                }
            })
            .map_err(|source| EmbeddingIoError::RegionWorkerStart { source })?;
        Ok(Self {
            sender: Some(sender),
            worker_thread: Some(worker_thread),
        })
    }

    async fn lookup(
        &self,
        environment: BTreeMap<String, String>,
        secrets: SecretLookup,
    ) -> Result<(Option<SecretString>, Option<SecretString>), EmbeddingIoError> {
        let (answer, result) = oneshot::channel();
        let job: RegionJob = Box::new(move || {
            drop(answer.send(region_secrets(&environment, &secrets)));
        });
        let Some(sender) = self.sender.as_ref() else {
            return Err(EmbeddingIoError::RegionWorkerUnavailable);
        };
        sender
            .send(job)
            .await
            .map_err(|_closed| EmbeddingIoError::RegionWorkerUnavailable)?;
        result
            .await
            .map_err(|_closed| EmbeddingIoError::RegionWorkerUnavailable)?
    }
}

impl Drop for ManagedRegionWorker {
    fn drop(&mut self) {
        drop(self.sender.take());
        if let Some(worker_thread) = self.worker_thread.take()
            && let Err(_panic) = worker_thread.join()
        {
            // A failed secret reader must not unwind a service destructor.
        }
    }
}

pub struct CredentialRequest<'source> {
    pub optional_params: &'source mut BTreeMap<String, Box<RawValue>>,
    pub environment: &'source BTreeMap<String, String>,
    pub bearer_token: Option<&'source SecretString>,
}

#[derive(Debug, Clone)]
pub struct LoadedCredentials {
    pub credentials: Option<Credentials>,
    pub region: String,
}

// LiteLLM pops the aws_* keys from the caller's options before it looks at the region
pub async fn load_credentials<Resolve, Fut>(
    request: CredentialRequest<'_>,
    secrets: &SecretLookup,
    region_worker: &ManagedRegionWorker,
    resolve: Resolve,
) -> Result<LoadedCredentials, EmbeddingIoError>
where
    Resolve: FnOnce(&AwsSettings, &str) -> Fut,
    Fut: Future<Output = Result<Credentials, CredentialError>>,
{
    let auth = pop_aws_auth_params(request.optional_params)?;
    let region = match explicit_region(request.optional_params)? {
        Some(region) => region,
        None => environment_region(request.environment, secrets, region_worker).await?,
    };
    if request.bearer_token.is_some() {
        return Ok(LoadedCredentials {
            credentials: None,
            region,
        });
    }
    let settings = auth_settings(auth, &region);
    let credentials = resolve(&settings, &region).await?;
    Ok(LoadedCredentials {
        credentials: Some(credentials),
        region,
    })
}

// an explicit region is used as given: this helper starts no validation and no env lookup for it
fn explicit_region(
    parameters: &mut BTreeMap<String, Box<RawValue>>,
) -> Result<Option<String>, EmbeddingIoError> {
    let Some(raw) = parameters.remove("aws_region_name") else {
        return Ok(None);
    };
    serde_json::from_str::<Option<String>>(raw.get())
        .map_err(|source| EmbeddingIoError::Region { source })
}

// AWS_REGION overrides AWS_REGION_NAME, and a value that is not a string leaves the other lookup standing
async fn environment_region(
    environment: &BTreeMap<String, String>,
    secrets: &SecretLookup,
    region_worker: &ManagedRegionWorker,
) -> Result<String, EmbeddingIoError> {
    let (name, standard) = if secrets.manager.is_some() {
        region_worker
            .lookup(environment.clone(), secrets.clone())
            .await?
    } else {
        region_secrets(environment, secrets)?
    };
    Ok(standard.or(name).map_or_else(
        || DEFAULT_AWS_REGION.to_owned(),
        |text| text.expose_secret().to_owned(),
    ))
}

fn region_secrets(
    environment: &BTreeMap<String, String>,
    secrets: &SecretLookup,
) -> Result<(Option<SecretString>, Option<SecretString>), EmbeddingIoError> {
    let name = secrets.get_secret_str(environment, REGION_NAME_VARIABLE, None)?;
    let standard = secrets.get_secret_str(environment, REGION_VARIABLE, None)?;
    Ok((name, standard))
}
