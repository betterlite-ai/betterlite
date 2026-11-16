use betterlite_config::sections::foundry_credentials::Settings as FoundrySettings;
use betterlite_config::sections::foundry_image_edits::Settings as VersionSettings;

use crate::credentials::model_info::ModelInfoAccessors;
use crate::image_edits::error::ImageEditError;
use crate::image_edits::url::{CompleteUrlRequest, complete_url};

use super::support::{self, FLUX1, FLUX2, MAI};

const BASE: &str = "https://res.services.ai.azure.com";

struct Env {
    foundry: FoundrySettings,
    version: VersionSettings,
}

impl Env {
    fn new(base: Option<&str>, version: Option<&str>) -> Self {
        Self {
            foundry: support::foundry_settings(base, None),
            version: support::version_settings(version),
        }
    }
}

fn url(
    model: &str,
    api_base: Option<&str>,
    api_version: Option<&str>,
    global_api_version: Option<&str>,
    env: &Env,
) -> Result<String, ImageEditError> {
    complete_url(&CompleteUrlRequest {
        model,
        accessors: ModelInfoAccessors {
            global_api_base: None,
            global_api_key: None,
            environment_api_base: env.foundry.api_base.as_deref(),
            environment_api_key: None,
        },
        api_base,
        api_version,
        global_api_version,
        environment: &env.version,
    })
}

pub(super) fn image_edit_api_version() {
    version_placement();
    version_precedence();
    caller_base_wins();
}

fn version_placement() {
    let deployment = Env::new(
        Some("https://res.openai.azure.com/openai/deployments/"),
        None,
    );
    assert_eq!(
        url(FLUX1, None, Some("2025-04-01-preview"), None, &deployment).unwrap(),
        "https://res.openai.azure.com/openai/deployments/images/edits?api-version=2025-04-01-preview"
    );

    // a base without the deployment path gets the model's deployment added
    let account = Env::new(Some(BASE), None);
    assert_eq!(
        url(FLUX1, None, Some("v1"), None, &account).unwrap(),
        "https://res.services.ai.azure.com/openai/deployments/FLUX-1-kontext-pro/images/edits?api-version=v1"
    );
    let both = Env::new(
        Some("https://res.openai.azure.com/openai/deployments"),
        None,
    );
    assert_eq!(
        url(FLUX1, None, Some("v1"), None, &both).unwrap(),
        "https://res.openai.azure.com/openai/deployments/FLUX-1-kontext-pro/images/edits?api-version=v1"
    );

    // the version is encoded, and an existing query is replaced rather than merged
    let query = Env::new(Some("https://res.services.ai.azure.com/api?x=1"), None);
    assert_eq!(
        url(FLUX1, None, Some("v 1"), None, &query).unwrap(),
        "https://res.services.ai.azure.com/api/openai/deployments/FLUX-1-kontext-pro/images/edits?api-version=v+1"
    );
}

fn version_precedence() {
    let full = Env::new(Some(BASE), Some("from-env"));
    for (api_version, global, expected) in [
        (Some("request"), Some("global"), "request"),
        (None, Some("global"), "global"),
        (None, None, "from-env"),
    ] {
        let built = url(FLUX1, None, api_version, global, &full).unwrap();
        let (_, version) = built.rsplit_once("api-version=").expect("a version");
        assert_eq!(version, expected);
    }
}

fn caller_base_wins() {
    let env = Env::new(Some("https://operator.test"), Some("from-env"));
    assert_eq!(
        url(FLUX1, Some("https://caller.test"), Some("v1"), None, &env).unwrap(),
        "https://caller.test/openai/deployments/FLUX-1-kontext-pro/images/edits?api-version=v1"
    );
}

pub(super) fn flux1_missing_version() {
    let env = Env::new(Some(BASE), None);
    let error = url(FLUX1, None, None, None, &env).unwrap_err();
    assert!(matches!(error, ImageEditError::ApiVersionRequired));
    assert_eq!(error.status_code(), None);
    assert!(error.to_string().contains("AZURE_AI_API_VERSION"));

    // an explicit empty string is a value, so it is used as given instead of counting as missing
    assert_eq!(
        url(FLUX1, None, Some(""), None, &env).unwrap(),
        "https://res.services.ai.azure.com/openai/deployments/FLUX-1-kontext-pro/images/edits?api-version="
    );
    assert_eq!(
        url(FLUX1, None, None, Some(""), &env).unwrap(),
        "https://res.services.ai.azure.com/openai/deployments/FLUX-1-kontext-pro/images/edits?api-version="
    );
}

pub(super) fn flux2_preview_default() {
    let env = Env::new(Some(BASE), None);
    assert_eq!(
        url(FLUX2, None, None, None, &env).unwrap(),
        "https://res.services.ai.azure.com/providers/blackforestlabs/v1/flux-2-pro?api-version=preview"
    );
    assert_eq!(
        url(FLUX2, None, None, Some("2025-04-01-preview"), &env).unwrap(),
        "https://res.services.ai.azure.com/providers/blackforestlabs/v1/flux-2-pro?api-version=2025-04-01-preview"
    );
    let flex = Env::new(Some(BASE), Some("env"));
    assert_eq!(
        url("FLUX-2-flex", None, Some("request"), Some("global"), &flex).unwrap(),
        "https://res.services.ai.azure.com/providers/blackforestlabs/v1/flux-2-flex?api-version=request"
    );
    assert_eq!(
        url("FLUX-2-flex", None, None, None, &flex).unwrap(),
        "https://res.services.ai.azure.com/providers/blackforestlabs/v1/flux-2-flex?api-version=env"
    );

    // a providers base is used as it stands
    let providers = Env::new(
        Some("https://res.services.ai.azure.com/providers/blackforestlabs/v1/flux-2-pro"),
        None,
    );
    assert_eq!(
        url(FLUX2, None, None, None, &providers).unwrap(),
        "https://res.services.ai.azure.com/providers/blackforestlabs/v1/flux-2-pro?api-version=preview"
    );

    let error = url(FLUX2, None, None, None, &Env::new(None, None)).unwrap_err();
    assert!(matches!(error, ImageEditError::ApiBaseRequired));
}

pub(super) fn mai_preview_ignores_global() {
    let env = Env::new(Some(BASE), None);
    assert_eq!(
        url(MAI, None, None, Some("2024-02-01"), &env).unwrap(),
        "https://res.services.ai.azure.com/mai/v1/images/edits?api-version=preview"
    );
    assert_eq!(
        url(MAI, None, Some("2025-11-15"), Some("2024-02-01"), &env).unwrap(),
        "https://res.services.ai.azure.com/mai/v1/images/edits?api-version=2025-11-15"
    );
    let from_env = Env::new(Some(BASE), Some("2025-11-15"));
    assert_eq!(
        url(MAI, None, None, Some("2024-02-01"), &from_env).unwrap(),
        "https://res.services.ai.azure.com/mai/v1/images/edits?api-version=2025-11-15"
    );

    let error = url(MAI, None, None, None, &Env::new(None, None)).unwrap_err();
    assert!(matches!(error, ImageEditError::ApiBaseRequired));
    assert!(error.to_string().contains("Azure AI API base is required"));
}
