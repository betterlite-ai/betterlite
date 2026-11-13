use betterlite_http::url_path::encode_segment;

use super::API_VERSION;
use super::error::VertexInteractionsError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VertexInteractionsTarget {
    pub base_url: String,
    pub project_id: String,
    pub location: String,
}

impl VertexInteractionsTarget {
    #[must_use]
    pub fn collection_url(&self) -> String {
        format!(
            "{}/{API_VERSION}/projects/{}/locations/{}/interactions",
            self.base_url, self.project_id, self.location
        )
    }

    // the interaction id is one encoded path segment, and the project keeps the spelling the mint resolved
    pub fn interaction_url(&self, interaction_id: &str) -> Result<String, VertexInteractionsError> {
        let encoded = encode_segment(interaction_id, "interaction_id")
            .map_err(VertexInteractionsError::InteractionId)?;
        Ok(format!("{}/{encoded}", self.collection_url()))
    }
}
