use rust_decimal::Decimal;
use serde_json::value::RawValue;
use std::collections::BTreeMap;

pub mod nested;
use nested::apply_nested;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum PriceField {
    Input,
    Output,
    CacheCreation,
    CacheRead,
    AudioInput,
    AudioOutput,
    ImageInput,
    ImageOutput,
    VideoInput,
    VideoOutput,
    CharacterInput,
    PixelInput,
    QueryInput,
    RequestInput,
    SecondInput,
    SecondOutput,
    Annotation,
    Citation,
    CodeInterpreter,
    ComputerUse,
    FileSearch,
    VectorStore,
    SearchContext,
    GoogleMaps,
    Ocr,
    RegionalMultiplier,
    Guardrail,
    Other,
}

const PRICE_PREFIXES: &[(&str, PriceField)] = &[
    ("cache_creation_", PriceField::CacheCreation),
    ("cache_read_", PriceField::CacheRead),
    ("input_cost_per_audio", PriceField::AudioInput),
    ("output_cost_per_audio", PriceField::AudioOutput),
    ("input_cost_per_image", PriceField::ImageInput),
    ("output_cost_per_image", PriceField::ImageOutput),
    ("input_cost_per_video", PriceField::VideoInput),
    ("output_cost_per_video", PriceField::VideoOutput),
    ("annotation_", PriceField::Annotation),
    ("citation_", PriceField::Citation),
    ("code_interpreter_", PriceField::CodeInterpreter),
    ("computer_use_", PriceField::ComputerUse),
    ("file_search_", PriceField::FileSearch),
    ("vector_store_", PriceField::VectorStore),
    ("search_context_", PriceField::SearchContext),
    ("google_maps_", PriceField::GoogleMaps),
    ("ocr_", PriceField::Ocr),
    ("guardrail_", PriceField::Guardrail),
    ("input_", PriceField::Input),
    ("output_", PriceField::Output),
];

impl PriceField {
    fn from_wire(name: &str) -> Option<Self> {
        if name == "output_vector_size" {
            return None;
        }
        if let Some((_, field)) = PRICE_PREFIXES
            .iter()
            .find(|(prefix, _)| name.starts_with(prefix))
        {
            return Some(field.clone());
        }
        let contains = [
            ("character", Self::CharacterInput),
            ("pixel", Self::PixelInput),
            ("query", Self::QueryInput),
            ("request", Self::RequestInput),
            ("second", Self::SecondInput),
            ("multiplier", Self::RegionalMultiplier),
            ("uplift", Self::RegionalMultiplier),
        ];
        contains
            .iter()
            .find(|(part, _)| name.contains(part))
            .map(|(_, field)| field.clone())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PriceFields {
    pub values: BTreeMap<PriceField, BTreeMap<String, Decimal>>,
    pub provider_specific: ProviderSpecificPricing,
    pub guardrail_units: BTreeMap<String, Decimal>,
    pub metadata: MetadataPricing,
    pub sample_spec: SamplePricing,
    pub off_peak: Option<OffPeakPricing>,
    pub search_context: BTreeMap<String, Decimal>,
    pub tiers: Vec<TierPricing>,
}
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ProviderSpecificPricing {
    pub us: Option<Decimal>,
    pub fast: Option<Decimal>,
}
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MetadataPricing {
    pub per_hour: Option<Decimal>,
    pub per_minute: Option<Decimal>,
}
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SamplePricing {
    pub computer_use_input: Option<Decimal>,
    pub computer_use_output: Option<Decimal>,
    pub file_search_calls: Option<Decimal>,
    pub file_search_storage: Option<Decimal>,
    pub vector_store_storage: Option<Decimal>,
}
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OffPeakPricing {
    pub values: BTreeMap<String, Decimal>,
    pub windows: Option<Vec<nested::PriceWindow>>,
}
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TierPricing {
    pub values: BTreeMap<String, Decimal>,
    pub range: Vec<Decimal>,
}

#[derive(Debug, thiserror::Error)]
pub enum PriceFieldError {
    #[error("price field {path} must be a JSON number")]
    NotNumber { path: String },
    #[error("price field {path} is not a valid decimal: {value}: {source}")]
    InvalidDecimal {
        path: String,
        value: String,
        #[source]
        source: rust_decimal::Error,
    },
    #[error("price field {path} must be a JSON object: {source}")]
    NotObject {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("price field {path} must be a JSON array: {source}")]
    NotArray {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("unknown tier field {path}")]
    UnknownTier { path: String },
    #[error("price field {path} must not be null")]
    Null { path: String },
    #[error("price fields JSON is invalid: {source}")]
    Json { source: serde_json::Error },
}

impl PriceFields {
    pub fn parse(input: &RawValue) -> Result<Self, PriceFieldError> {
        let object: BTreeMap<String, Box<RawValue>> =
            serde_json::from_str(input.get()).map_err(|source| PriceFieldError::Json { source })?;
        let mut parsed = Self {
            values: BTreeMap::new(),
            provider_specific: ProviderSpecificPricing::default(),
            guardrail_units: BTreeMap::new(),
            metadata: MetadataPricing::default(),
            sample_spec: SamplePricing::default(),
            off_peak: None,
            search_context: BTreeMap::new(),
            tiers: Vec::new(),
        };
        for (name, value) in object {
            if let Some(field) = PriceField::from_wire(&name)
                && !matches!(
                    name.as_str(),
                    "guardrail_cost_per_unit"
                        | "provider_specific_entry"
                        | "metadata"
                        | "sample_spec"
                        | "off_peak_pricing"
                        | "search_context_cost_per_query"
                        | "tiered_pricing"
                )
            {
                parsed
                    .values
                    .entry(field)
                    .or_default()
                    .insert(name.clone(), decimal_at(&name, value.as_ref())?);
            }
            apply_nested(&mut parsed, &name, value.as_ref())?;
        }
        Ok(parsed)
    }
}

pub(super) fn decimal_at(path: &str, value: &RawValue) -> Result<Decimal, PriceFieldError> {
    let lexeme = value.get();
    if lexeme == "null" {
        return Err(PriceFieldError::Null {
            path: path.to_owned(),
        });
    }
    if lexeme.starts_with('"')
        || lexeme == "true"
        || lexeme == "false"
        || lexeme.starts_with('[')
        || lexeme.starts_with('{')
    {
        return Err(PriceFieldError::NotNumber {
            path: path.to_owned(),
        });
    }
    let decimal = if lexeme.contains(['e', 'E']) {
        Decimal::from_scientific(lexeme)
    } else {
        Decimal::from_str_exact(lexeme)
    };
    decimal.map_err(|source| PriceFieldError::InvalidDecimal {
        path: path.to_owned(),
        value: lexeme.to_owned(),
        source,
    })
}

pub(super) fn decimal_map(
    path: &str,
    value: &RawValue,
) -> Result<BTreeMap<String, Decimal>, PriceFieldError> {
    let object: BTreeMap<String, Box<RawValue>> =
        serde_json::from_str(value.get()).map_err(|source| PriceFieldError::NotObject {
            path: path.to_owned(),
            source,
        })?;
    object
        .into_iter()
        .map(|(key, raw_value)| {
            let child = format!("{path}.{key}");
            Ok((key, decimal_at(&child, raw_value.as_ref())?))
        })
        .collect()
}

mod tests;
