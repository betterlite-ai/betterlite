use std::slice::from_ref;

use betterlite_auth::auth_model_lists::credential::CredentialSnapshot;
use betterlite_models::decisions::Json;

use crate::deployment_identity::spec::DeploymentSpec;
use crate::model_catalog_lookup::lookup::CatalogLookup;
use crate::responses_affinity::boundary::encryption_boundary;
use crate::responses_affinity::config::{AffinitySettings, ENABLED_FLAG};
use crate::responses_affinity::error::AffinityError;
use crate::responses_affinity::history::{FilterRequest, History, MetadataSlot};
use crate::responses_affinity::identity::find_by_model_id;
use crate::responses_affinity::markers::{anthropic_model_id, input_model_id};
use crate::responses_affinity::strip::{strip_all, strip_for_targets};

// EncryptedContentAffinityCheck: pin a follow-up request to the deployment that minted its encrypted reasoning.
pub struct AffinityCheck<'check, 'catalog> {
    settings: AffinitySettings,
    router: Option<&'check CatalogLookup<'catalog>>,
    credentials: &'check CredentialSnapshot,
}

impl<'check, 'catalog> AffinityCheck<'check, 'catalog> {
    // EncryptedContentAffinityCheck.__init__: the registrar resolves the settings and injects the catalog and credentials.
    #[must_use]
    pub const fn new(
        settings: AffinitySettings,
        router: Option<&'check CatalogLookup<'catalog>>,
        credentials: &'check CredentialSnapshot,
    ) -> Self {
        Self {
            settings,
            router,
            credentials,
        }
    }

    // async_filter_deployments: the healthy pool narrowed to the origin, or to its encryption-boundary peers.
    #[must_use]
    pub fn filter(&self, request: FilterRequest<'_>) -> Result<Vec<DeploymentSpec>, AffinityError> {
        let FilterRequest {
            model,
            deployments,
            mut history,
        } = request;
        if !self.settings.enabled_for_group(model) {
            return Ok(deployments.to_vec());
        }
        flag_enabled(&mut history)?;
        let Some(model_id) = request_marker(&history) else {
            return Ok(deployments.to_vec());
        };
        tracing::debug!(
            "EncryptedContentAffinityCheck: decoded model_id={model_id} from the request's encrypted content markers"
        );
        if let Some(deployment) = find_by_model_id(deployments, &model_id) {
            tracing::debug!("EncryptedContentAffinityCheck: pinning -> deployment={model_id}");
            *history.pinned = true;
            strip_for_targets(
                &mut history,
                from_ref(deployment),
                self.router,
                self.credentials,
            )?;
            return Ok(vec![deployment.clone()]);
        }
        let peers = self.boundary_peers(deployments, &model_id)?;
        if peers.is_empty() {
            return Ok(fallback(model, &model_id, deployments, &mut history));
        }
        let peer_count = peers.len();
        tracing::debug!(
            "EncryptedContentAffinityCheck: model_id={model_id} not in healthy_deployments; pinning to {peer_count} deployment(s) on same encryption boundary"
        );
        *history.pinned = true;
        strip_for_targets(&mut history, &peers, self.router, self.credentials)?;
        Ok(peers)
    }

    // _find_deployments_on_same_encryption_boundary: the healthy peers of the catalogued origin, in pool order.
    fn boundary_peers(
        &self,
        deployments: &[DeploymentSpec],
        model_id: &str,
    ) -> Result<Vec<DeploymentSpec>, AffinityError> {
        let Some(router) = self.router else {
            return Ok(Vec::new());
        };
        let Some(origin) = router.get_deployment(model_id)? else {
            return Ok(Vec::new());
        };
        let Some(boundary) = encryption_boundary(&origin.litellm_params, self.credentials) else {
            return Ok(Vec::new());
        };
        let peers = deployments
            .iter()
            .filter(|deployment| {
                encryption_boundary(&deployment.litellm_params, self.credentials)
                    .is_some_and(|candidate| candidate == boundary)
            })
            .cloned()
            .collect();
        Ok(peers)
    }
}

// The flag the response post-processor reads; a missing litellm_metadata stays missing so tag routing keeps its dict.
fn flag_enabled(history: &mut History<'_>) -> Result<(), AffinityError> {
    match &mut history.metadata {
        MetadataSlot::Object(fields) => {
            fields.insert(ENABLED_FLAG.to_owned(), Json::Bool(true));
            Ok(())
        }
        MetadataSlot::Absent => Ok(()),
        MetadataSlot::NotAnObject { python_type } => Err(match *python_type {
            "list" => AffinityError::MetadataListNotAssignable,
            name => AffinityError::MetadataNotAssignable { python_type: name },
        }),
    }
}

// The origin cannot serve this turn and no peer shares its boundary: strip its reasoning and dispatch the pool.
fn fallback(
    model: &str,
    model_id: &str,
    deployments: &[DeploymentSpec],
    history: &mut History<'_>,
) -> Vec<DeploymentSpec> {
    let shown: String = model_id.chars().take(64).collect();
    tracing::warn!(
        "EncryptedContentAffinityCheck: model_id={shown} cannot serve group {model} and no deployment on the same encryption boundary is configured; forwarding without its encrypted reasoning"
    );
    strip_all(history);
    deployments.to_vec()
}

// The marker scan: the input's first encoded marker, then `or` the Anthropic bridge tags as Python reads them.
fn request_marker(history: &History<'_>) -> Option<String> {
    let from_input = input_model_id(history.input.as_deref());
    let marker = match from_input {
        Some(marker) if !marker.is_empty() => Some(marker),
        Some(_) | None => anthropic_model_id(history.effective_messages().unwrap_or_default()),
    };
    marker.filter(|marker| !marker.is_empty())
}
