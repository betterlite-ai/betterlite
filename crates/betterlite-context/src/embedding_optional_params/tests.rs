#![cfg(test)]

use std::sync::Mutex;

use crate::embedding_optional_params::request::EmbeddingRequest;
use crate::non_default_params::Params;
use crate::non_default_params::error::MapperError;
use crate::non_default_params::mapper::{DropPolicy, OptionalParamsMapper};

mod bedrock;
mod branches;
mod config_path;
mod dimensions;

fn params(text: &str) -> Params {
    serde_json::from_str(text).unwrap()
}

fn parse_pair(passed: &str, special: &str) -> (Params, Params) {
    (params(passed), params(special))
}

fn base<'case>(
    model: &'case str,
    provider: &'case str,
    passed: &'case Params,
    special: &'case Params,
) -> EmbeddingRequest<'case> {
    EmbeddingRequest {
        model,
        custom_llm_provider: provider,
        passed_params: passed,
        special_params: special,
        additional_drop_params: &[],
        allowed_openai_params: &[],
        drop_policy: DropPolicy::default(),
        provider_config: None,
        mapper: None,
        supported: None,
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
struct MapperSeen {
    non_default: Params,
    optional: Params,
    model: String,
    drop_params: bool,
}

struct FixtureMapper {
    supported: Vec<String>,
    mapped: Params,
    fail_status: Option<u16>,
    seen: Mutex<Vec<MapperSeen>>,
}

impl FixtureMapper {
    fn mapper(supported: &[&str], mapped: &str) -> Self {
        Self {
            supported: supported.iter().map(ToString::to_string).collect(),
            mapped: params(mapped),
            fail_status: None,
            seen: Mutex::new(Vec::new()),
        }
    }

    fn seen(&self) -> Vec<MapperSeen> {
        self.seen.lock().unwrap().clone()
    }
}

impl OptionalParamsMapper for FixtureMapper {
    fn supported_openai_params(&self, _model: &str) -> Vec<String> {
        self.supported.clone()
    }

    fn map_openai_params(
        &self,
        non_default_params: &Params,
        optional_params: Params,
        model: &str,
        drop_params: bool,
    ) -> Result<Params, MapperError> {
        let mut seen = match self.seen.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        seen.push(MapperSeen {
            non_default: non_default_params.clone(),
            optional: optional_params.clone(),
            model: model.to_owned(),
            drop_params,
        });
        drop(seen);
        if let Some(status_code) = self.fail_status {
            return Err(MapperError {
                status_code,
                message: "fixture mapper failed".to_owned(),
            });
        }
        let mut mapped = optional_params;
        mapped.extend(self.mapped.clone());
        Ok(mapped)
    }
}
