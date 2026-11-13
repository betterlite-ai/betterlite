use serde::Serialize;

// every by-id interaction request carries the empty JSON object, which serde writes for a fieldless struct
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct EmptyBody {
    #[serde(skip)]
    marker: (),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LifecycleRequest {
    pub url: String,
    pub body: EmptyBody,
}
