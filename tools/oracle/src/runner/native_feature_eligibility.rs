use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Path;

use serde::Serialize;

use crate::OracleError;
use crate::case::Case;

#[derive(Serialize)]
struct ArtifactMarker {
    native_feature_eligibility: bool,
}

pub(super) fn child_environment(
    root: &Path,
    case: &Case,
    litellm_child: bool,
) -> Result<BTreeMap<String, String>, OracleError> {
    let mut environment = case.env.clone();
    if case.native_feature_eligibility && litellm_child {
        let fixture = root.join("tools/oracle/fixtures/native-feature-oracle");
        let fixture_path = fixture.to_str().ok_or_else(|| OracleError::Filesystem {
            path: fixture.clone(),
            source: io::Error::other("native feature fixture path is not UTF-8"),
        })?;
        environment.insert(
            "LITELLM_WORKER_STARTUP_HOOKS".to_owned(),
            "startup:set_premium_user".to_owned(),
        );
        environment.insert("PYTHONPATH".to_owned(), fixture_path.to_owned());
    }
    Ok(environment)
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
