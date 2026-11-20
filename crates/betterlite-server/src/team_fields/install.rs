use std::sync::Arc;

use betterlite_config::sections::team_fields::Settings as TeamFieldsSettings;
use betterlite_pyhost::call_error::CallError;

use crate::python_hooks::HooksError;
use crate::python_hooks::PythonHooks;
use crate::python_hooks::slots::HookSlot;
use crate::team_request_contracts::metadata::TeamMetadataFieldSchema;

use super::python_validator::PythonValidator;
use super::validation::{
    TeamMetadataSchemaRegistry, TeamMetadataValidator, TeamMetadataValidatorRegistry,
};

#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error("could not load custom_team_metadata_validate {value:?}: {source}")]
    Load {
        value: String,
        #[source]
        source: Box<HooksError>,
    },
    #[error("custom_team_metadata_validate {value:?} cannot be inspected for async-ness: {source}")]
    NotCallable {
        value: String,
        #[source]
        source: CallError,
    },
}

pub async fn install_team_metadata(
    hooks: &PythonHooks,
    settings: &TeamFieldsSettings,
    validators: &TeamMetadataValidatorRegistry,
    schemas: &TeamMetadataSchemaRegistry,
) -> Result<(), InstallError> {
    let validator: Option<Arc<dyn TeamMetadataValidator>> = match settings.validator.as_deref() {
        Some(value) => Some(load_validator(hooks, value).await?),
        None => None,
    };
    validators.set(validator);
    schemas.set(
        settings
            .schema
            .iter()
            .map(TeamMetadataFieldSchema::from)
            .collect(),
    );
    Ok(())
}

async fn load_validator(
    hooks: &PythonHooks,
    value: &str,
) -> Result<Arc<dyn TeamMetadataValidator>, InstallError> {
    let hook = hooks
        .load_hook(HookSlot::CustomTeamMetadataValidate, value)
        .await
        .map_err(|source| InstallError::Load {
            value: value.to_owned(),
            source: Box::new(source),
        })?;
    let validator = PythonValidator::load(hook).map_err(|source| InstallError::NotCallable {
        value: value.to_owned(),
        source,
    })?;
    Ok(Arc::new(validator))
}
