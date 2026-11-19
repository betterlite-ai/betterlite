use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceKind {
    File,
    Batch,
    FineTuningJob,
}

impl ResourceKind {
    #[must_use]
    pub const fn text(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Batch => "batch",
            Self::FineTuningJob => "fine-tuning job",
        }
    }
}

// every detail is the literal HTTPException detail LiteLLM raises, so a route only chooses the envelope
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum FileUploadOptionsError {
    #[error(
        "target_model_names is required when require_managed_files is enabled in litellm_settings. Provide one or more model aliases via the target_model_names form field (e.g. target_model_names=my-model-alias)."
    )]
    TargetModelNamesRequired,
    #[error(
        "model is not allowed when require_managed_files is enabled in litellm_settings. Uploads must go through managed files using target_model_names instead of the model parameter."
    )]
    ModelNotAllowed,
    #[error(
        "Raw provider {} ids cannot be used when require_managed_files is enabled in litellm_settings. Use the LiteLLM managed {} id returned when the {} was created.",
        .kind.text(),
        .kind.text(),
        .kind.text()
    )]
    RawProviderId { kind: ResourceKind },
    #[error("Managed resource ownership validation is unavailable.")]
    OwnershipCheckUnavailable,
    #[error("The caller does not have access to this managed {} id.", .kind.text())]
    AccessDenied { kind: ResourceKind },
    #[error("'model' must be a string")]
    ModelNotText,
    #[error("'target_model_names' must be a list of strings")]
    NamesNotText,
}

impl FileUploadOptionsError {
    #[must_use]
    pub const fn status(&self) -> u16 {
        match self {
            Self::TargetModelNamesRequired
            | Self::ModelNotAllowed
            | Self::RawProviderId { .. }
            | Self::ModelNotText
            | Self::NamesNotText => 400,
            Self::OwnershipCheckUnavailable => 500,
            Self::AccessDenied { .. } => 403,
        }
    }
}
