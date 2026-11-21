use std::collections::VecDeque;
use std::future::pending;

use async_trait::async_trait;
use betterlite_error::error::ErrorClass;
use betterlite_error::facts::ProviderErrorFacts;
use betterlite_error::status::status_error_class;
use betterlite_router::cooldown_decisions::failure::FailureClass;
use betterlite_router::retry_policy::eligibility::Availability;
use betterlite_router::router_core::error::RoutingError;
use betterlite_router::routing_retries::attempt::{
    AttemptDispatch, AttemptFailure, AttemptRequest, ProviderFailure,
};
use betterlite_router::simple_shuffle::error::ShuffleError;
use betterlite_router::simple_shuffle::{Candidate, pick};
use rand::Rng;
use tokio_util::sync::CancellationToken;

use super::wire::{Call, Deployment, Step};
use crate::router_probes::retry_policy::ErrorInput;

// the provider each attempt reaches: it answers the scripted outcome and records what the router told it
pub(super) struct Scripted<'script> {
    pub(super) deployments: &'script [Deployment],
    pub(super) steps: VecDeque<Step>,
    // the routing's own token, which a disconnect cancels as the caller going away does
    pub(super) routing: CancellationToken,
    pub(super) calls: Vec<Call>,
    // why the script could not run as written, reported once the routing stops
    pub(super) fault: Option<String>,
}

impl Scripted<'_> {
    async fn halt(&mut self, fault: String) -> Result<(), AttemptFailure> {
        self.fault = Some(fault);
        self.routing.cancel();
        pending().await
    }
}

#[async_trait]
impl AttemptDispatch for Scripted<'_> {
    type Response = ();

    async fn attempt<R>(
        &mut self,
        request: AttemptRequest<'_>,
        random: &mut R,
    ) -> Result<(), AttemptFailure>
    where
        R: Rng + Send,
    {
        let deployment = route(self.deployments, request.skipped_deployment_ids, random)
            .map_err(|source| AttemptFailure::Routing(Box::new(RoutingError::from(source))))?;
        self.calls.push(Call {
            deployment: deployment.id.clone(),
            attempted_retries: request.trail.attempted_retries,
            max_retries: request.trail.max_retries,
            breadcrumbs: request.trail.previous_models.len(),
        });
        let Some(step) = self.steps.pop_front() else {
            return self
                .halt("the router made more attempts than the case scripts".to_owned())
                .await;
        };
        if step.disconnect {
            self.routing.cancel();
            return pending().await;
        }
        let Some(error) = step.error else {
            return Ok(());
        };
        match provider_error(error) {
            Ok(class) => Err(AttemptFailure::Provider(Box::new(ProviderFailure {
                error: class,
                deployment_id: Some(deployment.id.clone()),
                deployment_num_retries: deployment.num_retries,
                retry_after: None,
            }))),
            Err(fault) => self.halt(fault).await,
        }
    }

    // cooldowns are off, so every deployment of the group stays healthy
    async fn availability(&mut self) -> Result<Availability, RoutingError> {
        Ok(Availability {
            healthy_deployments: self.deployments.len(),
            all_deployments: self.deployments.len(),
            context_window_fallbacks: None,
            content_policy_fallbacks: None,
            regular_fallbacks: None,
        })
    }
}

// simple-shuffle over the deployments the retries have not skipped
fn route<'script, R>(
    deployments: &'script [Deployment],
    skipped: &[String],
    random: &mut R,
) -> Result<&'script Deployment, ShuffleError>
where
    R: Rng,
{
    let open: Vec<&Deployment> = deployments
        .iter()
        .filter(|deployment| !skipped.contains(&deployment.id))
        .collect();
    // no deployment sets a weight, rpm or tpm, so simple-shuffle picks uniformly as random.choice does
    let candidates = vec![
        Candidate {
            weight: 0,
            rpm: 0,
            tpm: 0,
        };
        open.len()
    ];
    let index = pick(&candidates, random)?;
    open.get(index).copied().ok_or(ShuffleError::NoCandidates)
}

// the provider's status mapped as exception_type maps it, which must land on the class the case names
fn provider_error(input: ErrorInput) -> Result<ErrorClass, String> {
    let Some(status) = input.status else {
        return Err("a scripted provider error needs a status".to_owned());
    };
    let facts = ProviderErrorFacts {
        status_code: Some(status),
        status_code_is_synthesized: false,
        message: None,
        body: None,
        headers: None,
        response: None,
        litellm_response_headers: None,
        request: None,
    };
    let class = status_error_class(&facts, "probe failure", "openai", None, None)
        .ok_or_else(|| format!("status {status} maps to no provider error"))?;
    if FailureClass::from(&class) == input.class {
        Ok(class)
    } else {
        Err(format!("status {status} maps to another error class"))
    }
}
