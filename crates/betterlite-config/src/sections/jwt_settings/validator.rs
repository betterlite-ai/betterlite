use std::fmt;
use std::path::Path;

use thiserror::Error;

// what get_instance_fn receives: the custom_validate module reference and the config file it came from
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValidatorRequest<'request> {
    pub module: &'request str,
    pub config_file_path: Option<&'request Path>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReturnAnnotation {
    LiteralTrue,
    // the loader's rendering of any other hint, None when the validator has no return annotation
    Other(String),
}

impl fmt::Display for ReturnAnnotation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LiteralTrue => formatter.write_str("typing.Literal[True]"),
            Self::Other(rendered) => formatter.write_str(rendered),
        }
    }
}

#[derive(Debug, Error)]
pub enum LoadFailure {
    #[error("custom_validate {module} needs a validator loader and none is registered")]
    Unregistered { module: String },
    #[error(
        "Remote module loading (s3://, gcs://) is only permitted from the config-file load path. Register the module under ``litellm_settings`` in your config.yaml instead."
    )]
    RemoteWithoutConfigFile,
    #[error("Could not import {instance} from {module}")]
    Import { instance: String, module: String },
}

// B165 and B054 install the loader that imports the module; loading settings never calls the validator
pub trait ValidatorLoader {
    fn load(&self, request: &ValidatorRequest<'_>) -> Result<ReturnAnnotation, LoadFailure>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct UnregisteredLoader;

impl ValidatorLoader for UnregisteredLoader {
    fn load(&self, request: &ValidatorRequest<'_>) -> Result<ReturnAnnotation, LoadFailure> {
        Err(LoadFailure::Unregistered {
            module: request.module.to_owned(),
        })
    }
}
