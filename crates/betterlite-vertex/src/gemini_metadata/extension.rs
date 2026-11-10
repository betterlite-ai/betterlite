use betterlite_models::decisions::Json;

use crate::provider_metadata::redaction::VERTEX_METADATA_FIELDS;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MetadataField {
    Grounding,
    UrlContext,
    SafetyRatings,
    SafetyResults,
    Citation,
}

impl MetadataField {
    pub const ALL: [Self; 5] = [
        Self::Grounding,
        Self::UrlContext,
        Self::SafetyRatings,
        Self::SafetyResults,
        Self::Citation,
    ];

    // the array pattern fails to compile if the shared constant stops naming five fields
    #[must_use]
    pub const fn wire_name(self) -> &'static str {
        let [
            grounding,
            url_context,
            safety_ratings,
            safety_results,
            citation,
        ] = VERTEX_METADATA_FIELDS;
        match self {
            Self::Grounding => grounding,
            Self::UrlContext => url_context,
            Self::SafetyRatings => safety_ratings,
            Self::SafetyResults => safety_results,
            Self::Citation => citation,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MetadataExtension {
    pub grounding_metadata: Vec<Json>,
    pub url_context_metadata: Vec<Json>,
    pub safety_ratings: Vec<Json>,
    pub safety_results: Vec<Json>,
    pub citation_metadata: Vec<Json>,
}

impl MetadataExtension {
    #[must_use]
    pub fn values(&self, field: MetadataField) -> &[Json] {
        match field {
            MetadataField::Grounding => &self.grounding_metadata,
            MetadataField::UrlContext => &self.url_context_metadata,
            MetadataField::SafetyRatings => &self.safety_ratings,
            MetadataField::SafetyResults => &self.safety_results,
            MetadataField::Citation => &self.citation_metadata,
        }
    }

    pub(crate) const fn values_mut(&mut self, field: MetadataField) -> &mut Vec<Json> {
        match field {
            MetadataField::Grounding => &mut self.grounding_metadata,
            MetadataField::UrlContext => &mut self.url_context_metadata,
            MetadataField::SafetyRatings => &mut self.safety_ratings,
            MetadataField::SafetyResults => &mut self.safety_results,
            MetadataField::Citation => &mut self.citation_metadata,
        }
    }

    // the hidden copies and assembled stream chunks skip a list that stayed empty
    #[must_use]
    pub fn populated_fields(&self) -> Vec<(&'static str, &[Json])> {
        MetadataField::ALL
            .into_iter()
            .map(|field| (field.wire_name(), self.values(field)))
            .filter(|(_, values)| !values.is_empty())
            .collect()
    }
}
