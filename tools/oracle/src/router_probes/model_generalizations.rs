use std::error::Error as StdError;

use betterlite_models::catalog_generalizations::ModelInfo;
use betterlite_models::decisions::{Extra, Json};
use betterlite_models::error::ModelsError;
use betterlite_router::generalization_integration::bounds::MAX_RULES_BYTES;
use betterlite_router::generalization_integration::error::{
    GeneralizationError, GeneralizationWarning,
};
use betterlite_router::generalization_integration::model_generalizations::ModelGeneralizations;
use serde::{Deserialize, Serialize};

use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

const EMPTY_DOCUMENT: &str = "[]";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    #[serde(default)]
    pub rules: Vec<Json>,
    #[serde(default)]
    pub queries: Vec<Query>,
    #[serde(default)]
    pub parse: Option<ParsePayload>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParsePayload {
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "query", rename_all = "snake_case", deny_unknown_fields)]
pub enum Query {
    Routing {
        model: String,
    },
    Capabilities {
        model: String,
    },
    FillMissing {
        model: String,
        provider: String,
    },
    Provider {
        model: String,
        #[serde(default)]
        exact: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Output {
    pub warnings: Vec<GeneralizationWarning>,
    pub engine_warnings: Vec<String>,
    pub results: Vec<Answer>,
    pub parse: Option<ParseOutput>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Answer {
    Routing {
        provider: Option<String>,
        warnings: Vec<GeneralizationWarning>,
    },
    Capabilities {
        model_info: Option<Json>,
        warnings: Vec<GeneralizationWarning>,
    },
    FillMissing {
        model_info: Option<Json>,
        warnings: Vec<GeneralizationWarning>,
    },
    Provider {
        provider: Option<String>,
        warnings: Vec<GeneralizationWarning>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ParseOutput {
    Valid { rules: usize },
    Invalid { bytes: usize, limit: usize },
}

fn invalid(error: &dyn StdError) -> OperationError {
    OperationError::InvalidRequest {
        message: error.to_string(),
    }
}

fn probe_info(model_info: Option<ModelInfo>) -> Result<Option<Json>, OperationError> {
    let Some(info) = model_info else {
        return Ok(None);
    };
    let mut fields = Extra::new();
    for (key, value) in info {
        let parsed: Json = serde_json::from_str(value.get()).map_err(|error| invalid(&error))?;
        fields.insert(key, parsed);
    }
    Ok(Some(Json::Object(fields)))
}

fn answer(service: &ModelGeneralizations, query: &Query) -> Result<Answer, OperationError> {
    match query {
        Query::Routing { model } => {
            let found = service.routing(model);
            Ok(Answer::Routing {
                provider: found.answer,
                warnings: found.warnings,
            })
        }
        Query::Capabilities { model } => {
            let found = service.capabilities(model);
            Ok(Answer::Capabilities {
                model_info: probe_info(found.answer)?,
                warnings: found.warnings,
            })
        }
        Query::FillMissing { model, provider } => {
            let found = service.fill_missing(model, provider);
            Ok(Answer::FillMissing {
                model_info: probe_info(found.answer)?,
                warnings: found.warnings,
            })
        }
        Query::Provider { model, exact } => {
            let found = service.provider(model, exact.as_deref());
            Ok(Answer::Provider {
                provider: found.answer,
                warnings: found.warnings,
            })
        }
    }
}

fn parse_step(payload: &ParsePayload) -> Result<ParseOutput, OperationError> {
    let length = usize::try_from(payload.bytes).map_err(|error| invalid(&error))?;
    if length < EMPTY_DOCUMENT.len() {
        return Err(OperationError::InvalidRequest {
            message: "the parse payload is shorter than an empty rule list".to_owned(),
        });
    }
    if length > MAX_RULES_BYTES.saturating_add(1) {
        return Err(OperationError::InvalidRequest {
            message: "the parse payload length is past the parser bound".to_owned(),
        });
    }
    let mut document = String::from(EMPTY_DOCUMENT);
    document.push_str(&" ".repeat(length.saturating_sub(EMPTY_DOCUMENT.len())));
    match ModelGeneralizations::install(document.as_bytes()) {
        Ok(installed) => Ok(ParseOutput::Valid {
            rules: installed.engine.rules.len(),
        }),
        Err(GeneralizationError::Rules(ModelsError::RulesTooLarge { size })) => {
            Ok(ParseOutput::Invalid {
                bytes: size,
                limit: MAX_RULES_BYTES,
            })
        }
        Err(error) => Err(invalid(&error)),
    }
}

pub(crate) fn run(
    input: &Input,
    _random: &mut InjectedRandom<'_>,
) -> Result<ProbeOutput, OperationError> {
    let bytes = serde_json::to_vec(&input.rules).map_err(|error| invalid(&error))?;
    let service = ModelGeneralizations::install(&bytes).map_err(|error| invalid(&error))?;
    let mut results = Vec::with_capacity(input.queries.len());
    for query in &input.queries {
        results.push(answer(&service, query)?);
    }
    let parse = input.parse.as_ref().map(parse_step).transpose()?;
    Ok(ProbeOutput::ModelGeneralizations(Output {
        warnings: service.warnings,
        engine_warnings: service.engine_warnings,
        results,
        parse,
    }))
}
