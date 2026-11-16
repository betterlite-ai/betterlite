use super::error::FactoryError;

pub(crate) enum Provider {
    S3,
    Vantage,
    Gcs,
    Mavvrik,
}

impl Provider {
    pub(crate) fn parse(name: &str) -> Result<Self, FactoryError> {
        let lowered = name.to_lowercase();
        match lowered.as_str() {
            "s3" => Ok(Self::S3),
            "vantage" => Ok(Self::Vantage),
            "gcs" => Ok(Self::Gcs),
            "mavvrik" => Ok(Self::Mavvrik),
            _ => Err(FactoryError::UnsupportedProvider { provider: lowered }),
        }
    }
}
