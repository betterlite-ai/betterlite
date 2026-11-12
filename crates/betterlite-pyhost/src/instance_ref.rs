use crate::instance_loader::LoadError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Storage {
    S3,
    Gcs,
}

impl Storage {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::S3 => "s3",
            Self::Gcs => "gcs",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstanceRef {
    Local {
        module_name: String,
        instance_name: String,
    },
    Remote(RemoteModule),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteModule {
    pub storage: Storage,
    pub bucket: String,
    pub object_key: String,
    pub module_path: String,
    pub instance_name: String,
}

#[derive(Debug, thiserror::Error)]
pub enum RefError {
    #[error("Invalid URL format: {url}. Expected: {scheme}://bucket-name/path/to/module.instance")]
    MissingPath { url: String, scheme: &'static str },
    #[error(
        "Invalid URL format in {url}. Don't include '.py' extension and you must specify the instance name. Expected format: {scheme}://{bucket}/{module_path}.instance_name (e.g., {scheme}://{bucket}/{module_path}.proxy_handler_instance)"
    )]
    PyExtension {
        url: String,
        scheme: &'static str,
        bucket: String,
        module_path: String,
    },
    #[error("Invalid module specification in {url}. Expected: path/to/module.instance_name")]
    MissingInstance { url: String },
}

impl InstanceRef {
    pub fn parse(value: &str) -> Result<Self, LoadError> {
        if let Some(rest) = value.strip_prefix("s3://") {
            return parse_remote(Storage::S3, value, rest);
        }
        if let Some(rest) = value.strip_prefix("gcs://") {
            return parse_remote(Storage::Gcs, value, rest);
        }
        // Python splits on every dot and joins all parts but the last as the module name
        let (module_name, instance_name) = value.rsplit_once('.').unwrap_or(("", value));
        Ok(Self::Local {
            module_name: module_name.to_owned(),
            instance_name: instance_name.to_owned(),
        })
    }
}

fn parse_remote(storage: Storage, url: &str, rest: &str) -> Result<InstanceRef, LoadError> {
    let Some((bucket, path_and_module)) = rest.split_once('/') else {
        return Err(LoadError::Reference(RefError::MissingPath {
            url: url.to_owned(),
            scheme: storage.as_str(),
        }));
    };
    if let Some(module_path) = path_and_module.strip_suffix(".py") {
        return Err(LoadError::Reference(RefError::PyExtension {
            url: url.to_owned(),
            scheme: storage.as_str(),
            bucket: bucket.to_owned(),
            module_path: module_path.to_owned(),
        }));
    }
    let Some((module_path, instance_name)) = path_and_module.rsplit_once('.') else {
        return Err(LoadError::Reference(RefError::MissingInstance {
            url: url.to_owned(),
        }));
    };
    Ok(InstanceRef::Remote(RemoteModule {
        storage,
        bucket: bucket.to_owned(),
        object_key: format!("{module_path}.py"),
        module_path: module_path.to_owned(),
        instance_name: instance_name.to_owned(),
    }))
}

mod tests;
