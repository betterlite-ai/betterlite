use indexmap::IndexMap;
use serde::Deserialize;

use super::error::{Marengo3Error, bad_request};
use super::fields::{
    Given, Parsed, choice, list, media_sources, number, segmentation as parse_segmentation, text,
};
use super::options::InputType;
use super::request::TimedMediaOptions;

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct RawParams {
    #[serde(rename = "inputType")]
    pub input_type_camel: Option<Given>,
    pub input_type: Option<Given>,
    pub media_source: Option<Given>,
    pub media_sources: Option<Given>,
    #[serde(rename = "bucketOwner")]
    pub bucket_owner: Option<Given>,
    #[serde(rename = "startSec")]
    pub start_sec: Option<Given>,
    #[serde(rename = "endSec")]
    pub end_sec: Option<Given>,
    pub segmentation: Option<Given>,
    #[serde(rename = "embeddingOption")]
    pub embedding_option: Option<Given>,
    #[serde(rename = "embeddingType")]
    pub embedding_type: Option<Given>,
    #[serde(rename = "embeddingScope")]
    pub embedding_scope: Option<Given>,
    #[serde(rename = "inferenceId")]
    pub inference_id: Option<Given>,
    #[serde(rename = "textTruncate")]
    pub text_truncate: Option<Given>,
    #[serde(rename = "lengthSec")]
    pub length_sec: Option<Given>,
    #[serde(rename = "useFixedLengthSec")]
    pub use_fixed_length_sec: Option<Given>,
    #[serde(rename = "minClipSec")]
    pub min_clip_sec: Option<Given>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Marengo3Params {
    pub input_type_camel: Option<InputType>,
    pub input_type: Option<InputType>,
    pub media_source: Option<String>,
    pub media_sources: Option<IndexMap<String, String>>,
    pub bucket_owner: Option<String>,
    pub timed: TimedMediaOptions,
    pub inference_id: Option<String>,
    pub given_2_7_only: Vec<&'static str>,
}

struct Issues(Vec<String>);

impl Issues {
    fn field<T>(
        &mut self,
        raw: Option<&Given>,
        location: &str,
        parse: fn(&Given, &str) -> Parsed<T>,
    ) -> Option<T> {
        match parse(raw?, location) {
            Ok(parsed) => Some(parsed),
            Err(lines) => {
                self.0.extend(lines);
                None
            }
        }
    }
}

fn mapping(raw: &Given, location: &str) -> Parsed<IndexMap<String, String>> {
    match raw {
        Given::Mapping(entries) => media_sources(entries, location),
        Given::Null | Given::Bool(_) | Given::Number(_) | Given::Text(_) | Given::List(_) => Err(
            vec![format!("{location}: Input should be a valid dictionary")],
        ),
    }
}

fn timed_options(raw: &RawParams, issues: &mut Issues) -> TimedMediaOptions {
    TimedMediaOptions {
        start_sec: issues.field(raw.start_sec.as_ref(), "startSec", number),
        end_sec: issues.field(raw.end_sec.as_ref(), "endSec", number),
        segmentation: issues.field(
            raw.segmentation.as_ref(),
            "segmentation",
            parse_segmentation,
        ),
        embedding_option: issues.field(
            raw.embedding_option.as_ref(),
            "embeddingOption",
            |value, at| list(value, at, choice),
        ),
        embedding_type: issues.field(raw.embedding_type.as_ref(), "embeddingType", |value, at| {
            list(value, at, choice)
        }),
        embedding_scope: issues.field(
            raw.embedding_scope.as_ref(),
            "embeddingScope",
            |value, at| list(value, at, choice),
        ),
    }
}

impl Marengo3Params {
    pub fn validate(raw: &RawParams) -> Result<Self, Marengo3Error> {
        let mut issues = Issues(Vec::new());
        let input_type_camel = issues.field(raw.input_type_camel.as_ref(), "inputType", choice);
        let input_type = issues.field(raw.input_type.as_ref(), "input_type", choice);
        let media_source = issues.field(raw.media_source.as_ref(), "media_source", text);
        let media_sources = issues.field(raw.media_sources.as_ref(), "media_sources", mapping);
        let bucket_owner = issues.field(raw.bucket_owner.as_ref(), "bucketOwner", text);
        let timed = timed_options(raw, &mut issues);
        let inference_id = issues.field(raw.inference_id.as_ref(), "inferenceId", text);
        if !issues.0.is_empty() {
            return Err(bad_request(format!(
                "Invalid Marengo 3.0 parameters: {}",
                issues.0.join("; ")
            )));
        }
        let given_2_7_only = [
            ("textTruncate", raw.text_truncate.is_some()),
            ("lengthSec", raw.length_sec.is_some()),
            ("useFixedLengthSec", raw.use_fixed_length_sec.is_some()),
            ("minClipSec", raw.min_clip_sec.is_some()),
        ]
        .into_iter()
        .filter_map(|(name, given)| given.then_some(name))
        .collect();
        Ok(Self {
            input_type_camel,
            input_type,
            media_source,
            media_sources,
            bucket_owner,
            timed,
            inference_id,
            given_2_7_only,
        })
    }

    #[must_use]
    pub fn resolved_input_type(&self) -> InputType {
        self.input_type_camel
            .or(self.input_type)
            .unwrap_or(InputType::Text)
    }
}

impl TimedMediaOptions {
    #[must_use]
    pub fn given(&self) -> Vec<&'static str> {
        [
            ("startSec", self.start_sec.is_some()),
            ("endSec", self.end_sec.is_some()),
            ("segmentation", self.segmentation.is_some()),
            ("embeddingOption", self.embedding_option.is_some()),
            ("embeddingType", self.embedding_type.is_some()),
            ("embeddingScope", self.embedding_scope.is_some()),
        ]
        .into_iter()
        .filter_map(|(name, given)| given.then_some(name))
        .collect()
    }
}
