use std::error::Error;
use std::path::{Path, PathBuf};

use crate::custom_interface::CustomSecretManager;
use crate::secret_types::KeyManagementSystem;

#[derive(Debug, thiserror::Error)]
pub enum CustomLoaderError<RuntimeError>
where
    RuntimeError: Error + Send + Sync + 'static,
{
    #[error("config_file_path is required")]
    MissingConfigPath,
    #[error("custom_secret_manager field is required")]
    MissingManager,
    #[error("custom_secret_manager must contain exactly one file.class separator")]
    InvalidClassPath,
    #[error("custom secret manager file was not found: {0}")]
    FileNotFound(PathBuf),
    #[error("{0} must be a subclass of CustomSecretManager")]
    WrongBase(String),
    #[error("custom secret manager runtime operation failed")]
    Runtime(#[source] RuntimeError),
}

pub trait CustomSecretManagerRuntime {
    type Error: Error + Send + Sync + 'static;

    fn is_custom_subclass(&mut self, module: &Path, class_name: &str) -> Result<bool, Self::Error>;
    fn instantiate_without_arguments(
        &mut self,
        module: &Path,
        class_name: &str,
    ) -> Result<Box<dyn CustomSecretManager>, Self::Error>;
    fn set_secret_manager_client(
        &mut self,
        client: Box<dyn CustomSecretManager>,
    ) -> Result<(), Self::Error>;
    fn set_key_management_system(&mut self, system: KeyManagementSystem)
    -> Result<(), Self::Error>;
}

pub fn load_custom_secret_manager<R>(
    config_file_path: Option<&Path>,
    custom_secret_manager: Option<&str>,
    runtime: &mut R,
) -> Result<(), CustomLoaderError<R::Error>>
where
    R: CustomSecretManagerRuntime,
{
    let config_path = config_file_path.ok_or(CustomLoaderError::MissingConfigPath)?;
    let manager_path = custom_secret_manager.ok_or(CustomLoaderError::MissingManager)?;
    let (file_name, class_name) = split_class_path::<R>(manager_path)?;
    let directory = config_path.parent().unwrap_or_else(|| Path::new("."));
    let module_path = directory.join(format!("{file_name}.py"));
    if !module_path.is_file() {
        return Err(CustomLoaderError::FileNotFound(module_path));
    }
    if !runtime
        .is_custom_subclass(&module_path, class_name)
        .map_err(CustomLoaderError::Runtime)?
    {
        return Err(CustomLoaderError::WrongBase(class_name.to_owned()));
    }
    let client = runtime
        .instantiate_without_arguments(&module_path, class_name)
        .map_err(CustomLoaderError::Runtime)?;
    runtime
        .set_secret_manager_client(client)
        .map_err(CustomLoaderError::Runtime)?;
    runtime
        .set_key_management_system(KeyManagementSystem::Custom)
        .map_err(CustomLoaderError::Runtime)
}

fn split_class_path<R>(path: &str) -> Result<(&str, &str), CustomLoaderError<R::Error>>
where
    R: CustomSecretManagerRuntime,
{
    let mut parts = path.split('.');
    let file_name = parts.next().unwrap_or_default();
    let class_name = parts.next().ok_or(CustomLoaderError::InvalidClassPath)?;
    if file_name.is_empty() || class_name.is_empty() || parts.next().is_some() {
        return Err(CustomLoaderError::InvalidClassPath);
    }
    Ok((file_name, class_name))
}

#[path = "custom_loader/tests.rs"]
mod tests;
