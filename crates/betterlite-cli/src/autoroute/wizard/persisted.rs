use std::fs;
use std::io::Write as _;
use std::path::Path;

use betterlite_logging::private_json::files::PrivateFiles;
use secrecy::SecretString;
use tokio::runtime::Builder;

use super::error::WizardError;
use crate::autoroute::config::document::{GeneratedConfigDocument, master_key_from_config};
use crate::autoroute::paths;
use crate::settings_backup::secure::secure_create;

pub(crate) fn persisted_master_key(home: &Path) -> Option<SecretString> {
    let text = fs::read_to_string(paths::config_path(home)).ok()?;
    let document: GeneratedConfigDocument = serde_saphyr::from_str(&text).ok()?;
    master_key_from_config(&document)
}

pub(crate) fn write_document(
    files: &PrivateFiles,
    home: &Path,
    document: &GeneratedConfigDocument,
) -> Result<(), WizardError> {
    let runtime = Builder::new_current_thread()
        .build()
        .map_err(WizardError::Runtime)?;
    runtime
        .block_on(files.ensure_private_dir(&paths::autoroute_dir(home)))
        .map_err(WizardError::Directory)?;
    let yaml = serde_saphyr::to_string(document).map_err(WizardError::Serialize)?;
    let path = paths::config_path(home);
    let mut file = secure_create(&path).map_err(|source| WizardError::Create {
        path: path.clone(),
        source: Box::new(source),
    })?;
    file.write_all(yaml.as_bytes())
        .map_err(|source| WizardError::Write { path, source })
}
