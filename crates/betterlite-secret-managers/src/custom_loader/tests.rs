#![cfg(test)]

use std::env::temp_dir;
use std::fs::{create_dir_all, remove_dir_all, write};
use std::future::ready;
use std::path::{Path, PathBuf};
use std::process::id;

use secrecy::SecretString;
use thiserror::Error;

use super::{CustomLoaderError, CustomSecretManagerRuntime, load_custom_secret_manager};
use crate::custom_interface::{
    CustomParams, CustomSecretError, CustomSecretFuture, CustomSecretManager,
};
use crate::secret_types::KeyManagementSystem;

enum ClassKind {
    Custom,
    Other,
}

enum Constructor {
    Works,
    Fails,
}

#[derive(Debug, Error)]
enum RuntimeError {
    #[error("constructor failed")]
    Constructor,
}

struct Runtime {
    class: ClassKind,
    constructor: Constructor,
    installed: bool,
    system: Option<KeyManagementSystem>,
}

impl CustomSecretManagerRuntime for Runtime {
    type Error = RuntimeError;

    fn is_custom_subclass(&mut self, _: &Path, _: &str) -> Result<bool, Self::Error> {
        Ok(matches!(self.class, ClassKind::Custom))
    }

    fn instantiate_without_arguments(
        &mut self,
        _: &Path,
        _: &str,
    ) -> Result<Box<dyn CustomSecretManager>, Self::Error> {
        match self.constructor {
            Constructor::Works => Ok(Box::new(Manager {
                name: "custom_secret_manager".to_owned(),
            })),
            Constructor::Fails => Err(RuntimeError::Constructor),
        }
    }

    fn set_secret_manager_client(
        &mut self,
        _: Box<dyn CustomSecretManager>,
    ) -> Result<(), Self::Error> {
        self.installed = true;
        Ok(())
    }

    fn set_key_management_system(
        &mut self,
        system: KeyManagementSystem,
    ) -> Result<(), Self::Error> {
        self.system = Some(system);
        Ok(())
    }
}

struct Manager {
    name: String,
}

impl CustomSecretManager for Manager {
    fn secret_manager_name(&self) -> &str {
        &self.name
    }

    fn async_read_secret<'borrowed>(
        &'borrowed self,
        _: &'borrowed str,
        _: Option<&'borrowed CustomParams>,
    ) -> CustomSecretFuture<'borrowed, Option<SecretString>> {
        Box::pin(ready(Ok(None)))
    }

    fn sync_read_secret(
        &self,
        _: &str,
        _: Option<&CustomParams>,
    ) -> Result<Option<SecretString>, CustomSecretError> {
        Ok(None)
    }
}

#[test]
fn custom_loader_behaviors() {
    let root = temp_dir().join(format!("betterlite-custom-manager-{}", id()));
    create_dir_all(&root).unwrap();
    let config_path = write_config(&root, "config.yaml", "general_settings: {}\n");
    write(root.join("custom.py"), "class Manager: pass\n").unwrap();
    assert_successful_install(&config_path);
    assert_required_values(&root);
    assert_class_loading_errors(&root, &config_path);
    remove_dir_all(&root).unwrap();
}

fn assert_successful_install(config_path: &Path) {
    let mut runtime = runtime(ClassKind::Custom, Constructor::Works);
    load_custom_secret_manager(Some(config_path), Some("custom.Manager"), &mut runtime).unwrap();
    assert!(runtime.installed);
    assert_eq!(runtime.system, Some(KeyManagementSystem::Custom));
}

fn assert_required_values(root: &Path) {
    let mut runtime = runtime(ClassKind::Custom, Constructor::Works);
    assert!(matches!(
        load_custom_secret_manager(None, Some("custom.Manager"), &mut runtime),
        Err(CustomLoaderError::MissingConfigPath)
    ));
    assert!(matches!(
        load_custom_secret_manager(Some(root), None, &mut runtime),
        Err(CustomLoaderError::MissingManager)
    ));
}

fn assert_class_loading_errors(root: &Path, config_path: &Path) {
    let invalid_path = write_config(root, "invalid-path.yaml", "unused: true\n");
    let mut runtime = runtime(ClassKind::Custom, Constructor::Works);
    assert!(matches!(
        load_custom_secret_manager(
            Some(&invalid_path),
            Some("custom.Manager.Inner"),
            &mut runtime
        ),
        Err(CustomLoaderError::InvalidClassPath)
    ));
    let missing_file = write_config(root, "missing-file.yaml", "unused: true\n");
    assert!(matches!(
        load_custom_secret_manager(Some(&missing_file), Some("absent.Manager"), &mut runtime),
        Err(CustomLoaderError::FileNotFound(_))
    ));
    assert_wrong_base(config_path);
    assert_constructor_error(config_path);
}

fn assert_wrong_base(config_path: &Path) {
    let mut runtime = runtime(ClassKind::Other, Constructor::Works);
    assert!(matches!(
        load_custom_secret_manager(Some(config_path), Some("custom.Manager"), &mut runtime),
        Err(CustomLoaderError::WrongBase(_))
    ));
}

fn assert_constructor_error(config_path: &Path) {
    let mut runtime = runtime(ClassKind::Custom, Constructor::Fails);
    let result =
        load_custom_secret_manager(Some(config_path), Some("custom.Manager"), &mut runtime);
    assert!(matches!(
        result,
        Err(CustomLoaderError::Runtime(RuntimeError::Constructor))
    ));
}

fn runtime(class: ClassKind, constructor: Constructor) -> Runtime {
    Runtime {
        class,
        constructor,
        installed: false,
        system: None,
    }
}

fn write_config(root: &Path, name: &str, body: &str) -> PathBuf {
    let path = root.join(name);
    write(&path, body).unwrap();
    path
}
