mod script;
mod steps;

use std::collections::BTreeMap;
use std::io;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_context::aws_partition::get_aws_partition;
use betterlite_secret_managers::aws::{AwsDependencies, AwsSecretsManager};
use betterlite_secret_managers::aws_secret_reads::AwsReadError;
use betterlite_secret_managers::aws_secrets::AwsSecretClient;
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::helpers::{Helper, HelperError, HelperResult};
use script::{Resolver, Route, Script};

pub(crate) struct Op;

pub(super) type Failure = AwsReadError<io::Error, io::Error>;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    env: BTreeMap<String, String>,
    manager: Manager,
    routes: Vec<Route>,
    steps: Vec<Step>,
}

#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct Manager {
    aws_region_name: Option<String>,
    replica_regions: Vec<String>,
    kms_key_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(tag = "op", deny_unknown_fields)]
enum Step {
    #[serde(rename = "async_write_secret")]
    WriteSecret {
        name: String,
        value: String,
        #[serde(default)]
        description: Option<String>,
        #[serde(default)]
        tags: Option<Value>,
    },
    #[serde(rename = "async_delete_secret")]
    DeleteSecret {
        name: String,
        #[serde(default)]
        recovery_window_in_days: Option<u32>,
    },
    #[serde(rename = "async_rotate_secret")]
    RotateSecret {
        current: String,
        new: String,
        value: String,
    },
    #[serde(rename = "async_replicate_secret")]
    ReplicateSecret { name: String, regions: Vec<String> },
    #[serde(rename = "async_put_secret_value")]
    PutSecretValue { name: String, value: String },
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .map_err(|error| {
                Box::new(HelperError {
                    class: "RuntimeError".to_owned(),
                    status: None,
                    error_type: None,
                    param: None,
                    code: None,
                    message: error.to_string(),
                })
            })?;
        Ok(runtime.block_on(execute(input)))
    }
}

async fn execute(input: &Input) -> Value {
    let script = Script::new(&input.routes);
    let manager = AwsSecretsManager {
        aws_region_name: input.manager.aws_region_name.clone(),
        aws_role_name: None,
        aws_session_name: None,
        aws_external_id: None,
        aws_profile_name: None,
        aws_web_identity_token: None,
        aws_sts_endpoint: None,
        replica_regions: input.manager.replica_regions.clone(),
        kms_key_id: input.manager.kms_key_id.clone(),
        dependencies: AwsDependencies {
            environment: input.env.clone(),
            transport: Arc::clone(&script),
            resolver: Resolver {
                environment: input.env.clone(),
            },
            dns_suffix: |region: &str| get_aws_partition(Some(region)).dns_suffix,
            clock: signing_time,
        },
    };
    let client = AwsSecretClient {
        manager: &manager,
        next_token: Uuid::new_v4,
    };
    let mut steps = Vec::new();
    for step in &input.steps {
        let result = steps::run_step(&client, step).await;
        steps.push(json!({"result": result, "requests": script.take_requests()}));
    }
    json!({"steps": steps})
}

fn signing_time() -> SystemTime {
    UNIX_EPOCH
        .checked_add(Duration::from_secs(1_709_641_845))
        .unwrap_or(UNIX_EPOCH)
}
