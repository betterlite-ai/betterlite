use thiserror::Error;

#[derive(Debug, Error)]
pub enum InitializerError {
    #[error("the Bedrock streaming parameter {name} has an invalid value")]
    InvalidStreamingParam { name: String },
    #[error("the guardrail parameter {} has an invalid value", .0)]
    InvalidParam(String),
    #[error(
        "Trying to use Secret DetectionMissing litellm-enterprise package. Please install it to use this feature. Run `pip install litellm-enterprise`"
    )]
    MissingEnterprisePackage,
    #[error("PANW Prisma AIRS: api_key is required")]
    MissingApiKey,
    #[error("PANW Prisma AIRS: profile_name is required")]
    MissingProfileName,
}
