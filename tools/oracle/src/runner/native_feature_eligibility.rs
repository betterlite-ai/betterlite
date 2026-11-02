use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde::Serialize;

use crate::OracleError;
use crate::case::Case;

#[derive(Serialize)]
struct ArtifactMarker {
    native_feature_eligibility: bool,
}

// Resolved beside the crate manifest, not the caller's artifact root, so it is cwd-independent.
const FIXTURE_DIRECTORY: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/fixtures/native-feature-oracle"
);

pub(super) fn child_environment(
    case: &Case,
    litellm_child: bool,
    upstream: &str,
) -> BTreeMap<String, String> {
    let mut environment = case
        .env
        .iter()
        .map(|(name, value)| (name.clone(), value.replace("{{upstream}}", upstream)))
        .collect::<BTreeMap<_, _>>();
    if case.native_feature_eligibility && litellm_child {
        environment.insert(
            "LITELLM_WORKER_STARTUP_HOOKS".to_owned(),
            "startup:set_premium_user".to_owned(),
        );
        environment.insert("PYTHONPATH".to_owned(), FIXTURE_DIRECTORY.to_owned());
    }
    environment
}

pub(super) fn write_marker(directory: &Path, eligible: bool) -> Result<(), OracleError> {
    let marker = ArtifactMarker {
        native_feature_eligibility: eligible,
    };
    let marker_path = directory.join("native-feature-eligibility.json");
    let marker_text = serde_json::to_string_pretty(&marker).map_err(OracleError::Json)?;
    fs::write(&marker_path, marker_text).map_err(|source| OracleError::Filesystem {
        path: marker_path,
        source,
    })?;
    Ok(())
}
