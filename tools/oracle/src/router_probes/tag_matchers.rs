use betterlite_router::tag_matchers::candidates::{DeploymentMatch, DeploymentTags, MatchVia};
use betterlite_router::tag_matchers::document::{MAX_TAG_MATCH_BYTES, parse_tag_match};
use betterlite_router::tag_matchers::error::TagMatcherError;
use betterlite_router::tag_matchers::patterns::TagWarning;
use betterlite_router::tag_matchers::selection::{TagMatcher, TagQuery, TagSelection};
use betterlite_router::tag_matchers::tokens::RequestTags;
use serde::{Deserialize, Serialize};

use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    #[serde(default)]
    pub deployments: Vec<DeploymentInput>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub inherited_tags: Option<Vec<String>>,
    #[serde(default)]
    pub header_strings: Vec<String>,
    #[serde(default = "default_match_any")]
    pub match_any: bool,
    #[serde(default)]
    pub routing_prefix: String,
    #[serde(default)]
    pub parse: Option<ParsePayload>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeploymentInput {
    pub id: String,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    pub tag_regex: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParsePayload {
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Output(Box<Fields>);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fields {
    pub tags: TagsOutput,
    pub allowed: Vec<String>,
    pub candidates: Vec<String>,
    pub matches: Vec<MatchOutput>,
    pub defaults: Vec<String>,
    pub known_tags: Vec<String>,
    pub unknown_required_hides_answer: bool,
    pub warnings: Vec<TagWarning>,
    pub parse: Option<ParseOutput>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TagsOutput {
    pub rewritten: Vec<String>,
    pub required: Vec<String>,
    pub positive: Vec<String>,
    pub excluded: Vec<String>,
    pub routing_confirmed: Vec<String>,
    pub inherited_required: Option<Vec<String>>,
    pub inherited_excluded: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MatchOutput {
    pub id: String,
    pub matched_via: MatchVia,
    pub matched_value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ParseOutput {
    Valid { deployments: usize },
    Invalid { bytes: usize, limit: usize },
}

const fn default_match_any() -> bool {
    true
}

pub(crate) fn run(
    input: &Input,
    _random: &mut InjectedRandom<'_>,
) -> Result<ProbeOutput, OperationError> {
    let matcher = build_matcher(input);
    let selection = matcher.select(&query(input));
    // build warnings are reported once per pattern, match warnings per attempted pattern
    let mut warnings = matcher.warnings;
    warnings.extend(selection.warnings.iter().cloned());
    let parse = input.parse.as_ref().map(parse_step).transpose()?;
    let fields = fields(input, selection, warnings, parse);
    Ok(ProbeOutput::TagMatchers(Output(Box::new(fields))))
}

fn build_matcher(input: &Input) -> TagMatcher {
    let deployments = input
        .deployments
        .iter()
        .map(|deployment| DeploymentTags {
            tags: deployment.tags.clone(),
            tag_regex: deployment.tag_regex.clone(),
        })
        .collect();
    TagMatcher::new(deployments)
}

fn query(input: &Input) -> TagQuery {
    TagQuery {
        request: RequestTags {
            caller: Some(input.tags.clone()),
            inherited: input.inherited_tags.clone(),
        },
        headers: input.header_strings.clone(),
        match_any: input.match_any,
        routing_prefix: input.routing_prefix.clone(),
    }
}

fn fields(
    input: &Input,
    selection: TagSelection,
    warnings: Vec<TagWarning>,
    parse: Option<ParseOutput>,
) -> Fields {
    Fields {
        tags: TagsOutput {
            rewritten: selection.rewritten,
            required: selection.tags.required,
            positive: selection.tags.positive,
            excluded: selection.tags.excluded,
            routing_confirmed: selection.routing_confirmed.into_iter().collect(),
            inherited_required: selection
                .inherited_required
                .map(|values| values.into_iter().collect()),
            inherited_excluded: selection
                .inherited_excluded
                .map(|values| values.into_iter().collect()),
        },
        allowed: ids(&input.deployments, &selection.allowed),
        candidates: ids(&input.deployments, &selection.candidates),
        matches: match_outputs(&input.deployments, &selection.matches),
        defaults: ids(&input.deployments, &selection.defaults),
        known_tags: selection.known_tags.into_iter().collect(),
        unknown_required_hides_answer: selection.unknown_required_hides_answer,
        warnings,
        parse,
    }
}

fn match_outputs(deployments: &[DeploymentInput], matches: &[DeploymentMatch]) -> Vec<MatchOutput> {
    let mut outputs = Vec::with_capacity(matches.len());
    for matched in matches {
        let id = deployments
            .get(matched.index)
            .map_or_else(String::new, |deployment| deployment.id.clone());
        outputs.push(MatchOutput {
            id,
            matched_via: matched.via,
            matched_value: matched.value.clone(),
        });
    }
    outputs
}

fn ids(deployments: &[DeploymentInput], indexes: &[usize]) -> Vec<String> {
    indexes
        .iter()
        .filter_map(|index| deployments.get(*index))
        .map(|deployment| deployment.id.clone())
        .collect()
}

fn parse_step(payload: &ParsePayload) -> Result<ParseOutput, OperationError> {
    let length =
        usize::try_from(payload.bytes).map_err(|error| OperationError::InvalidRequest {
            message: error.to_string(),
        })?;
    if length > MAX_TAG_MATCH_BYTES.saturating_add(1) {
        return Err(OperationError::InvalidRequest {
            message: "the parse payload length is past the parser bound".to_owned(),
        });
    }
    let mut document = String::from("{\"deployments\":[]}");
    let padding =
        length
            .checked_sub(document.len())
            .ok_or_else(|| OperationError::InvalidRequest {
                message: "the parse payload is shorter than an empty tag match document".to_owned(),
            })?;
    document.push_str(&" ".repeat(padding));
    match parse_tag_match(document.as_bytes()) {
        Ok(request) => Ok(ParseOutput::Valid {
            deployments: request.deployments.len(),
        }),
        Err(TagMatcherError::TooLarge { bytes, limit }) => {
            Ok(ParseOutput::Invalid { bytes, limit })
        }
        Err(error) => Err(OperationError::InvalidRequest {
            message: error.to_string(),
        }),
    }
}
