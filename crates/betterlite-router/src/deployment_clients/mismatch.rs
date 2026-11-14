use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::model_registration::JsonObject;
use strum::VariantArray as _;

// The provider families that read a credential param, so a deployment resolving elsewhere silently ignores it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CredentialFamily {
    Aws,
    Vertex,
}

const SCOPED_CREDENTIAL_PARAMS: [(&str, CredentialFamily); 11] = [
    ("aws_access_key_id", CredentialFamily::Aws),
    ("aws_profile_name", CredentialFamily::Aws),
    ("aws_region_name", CredentialFamily::Aws),
    ("aws_role_name", CredentialFamily::Aws),
    ("aws_secret_access_key", CredentialFamily::Aws),
    ("aws_session_name", CredentialFamily::Aws),
    ("aws_session_token", CredentialFamily::Aws),
    ("aws_web_identity_token", CredentialFamily::Aws),
    ("vertex_credentials", CredentialFamily::Vertex),
    ("vertex_location", CredentialFamily::Vertex),
    ("vertex_project", CredentialFamily::Vertex),
];

impl CredentialFamily {
    const fn prefixes(self) -> &'static [&'static str] {
        match self {
            Self::Aws => &["bedrock", "sagemaker"],
            Self::Vertex => &["vertex_ai"],
        }
    }

    fn providers(self) -> Vec<&'static str> {
        ProviderId::VARIANTS
            .iter()
            .map(|provider| <&'static str>::from(*provider))
            .filter(|slug| {
                self.prefixes()
                    .iter()
                    .any(|prefix| slug.starts_with(prefix))
            })
            .collect()
    }

    fn reads(self, resolved_provider: &str) -> bool {
        self.prefixes()
            .iter()
            .any(|prefix| resolved_provider.starts_with(prefix))
    }
}

// a deployment carrying one provider's credentials while resolving to another warns; an unknown provider is left alone
#[must_use]
pub fn credential_mismatch_warning(
    model_name: &str,
    litellm_params: &JsonObject,
    resolved_provider: Option<&str>,
) -> Option<String> {
    let Some(Json::Text(model)) = litellm_params.get("model") else {
        return None;
    };
    if model.is_empty() {
        return None;
    }
    let mut scoped: Vec<(&str, CredentialFamily)> = SCOPED_CREDENTIAL_PARAMS
        .iter()
        .copied()
        .filter(|(field, _family)| {
            litellm_params
                .get(*field)
                .is_some_and(|value| !matches!(value, Json::Null))
        })
        .collect();
    if scoped.is_empty() {
        return None;
    }
    let resolved = resolved_provider?;
    scoped.retain(|(_field, family)| !family.reads(resolved));
    if scoped.is_empty() {
        return None;
    }
    scoped.sort_unstable_by_key(|(field, _family)| *field);
    let mismatched: Vec<&str> = scoped.iter().map(|(field, _family)| *field).collect();
    let mut expected: Vec<&'static str> = scoped
        .iter()
        .flat_map(|(_field, family)| family.providers())
        .collect();
    expected.sort_unstable();
    expected.dedup();
    let first = expected.first().copied().unwrap_or_default();
    Some(format!(
        "Deployment '{model_name}' sets {mismatched:?} but 'model={model}' resolves to provider '{resolved}', which ignores them. Those params are read by {expected:?}, so this is usually a missing route prefix (e.g. '{first}/{model}'); as written the request goes to '{resolved}' and will fail on that provider's credentials."
    ))
}
