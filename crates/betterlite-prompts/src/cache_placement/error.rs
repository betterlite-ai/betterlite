use thiserror::Error;

#[derive(Debug, Error)]
pub enum PlacementError {
    #[error("cache_control_injection_points is not a list of injection points")]
    InjectionPoints {
        #[source]
        source: serde_json::Error,
    },
    #[error("an injection point could not be written back into the request")]
    PointEncoding {
        #[source]
        source: serde_json::Error,
    },
}
