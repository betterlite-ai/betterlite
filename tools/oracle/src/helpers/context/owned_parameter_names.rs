use crate::helpers::HelperError;
use betterlite_context::owned_parameter_names::{
    OwnedParameterNamesError, OwnedRoot, Owner, is_litellm_owned_kwarg, owned_names,
    owned_wire_names, wire, wire_names,
};
use serde::{Deserialize, Serialize};

mod tests;

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    WireNameProjection {
        name: String,
        owner: Owner,
        root: Option<OwnedRoot>,
    },
    OwnedNameRegistry {
        names: Vec<String>,
    },
    InternalPrefixMembership {
        names: Vec<String>,
    },
    SubsetRegistries,
}

#[derive(Debug, Clone, Serialize)]
struct Projection {
    wire: betterlite_context::owned_parameter_names::WireMetadata,
    names: Vec<String>,
    owned: Option<Vec<String>>,
}
#[derive(Debug, Clone, Serialize)]
struct Registry {
    names: Vec<String>,
    owned: Vec<bool>,
}
#[derive(Debug, Clone, Serialize)]
struct Membership {
    owned: Vec<bool>,
}
#[derive(Debug, Clone, Serialize)]
struct Subsets {
    agentic_loop: Vec<String>,
    bedrock_batch: Vec<String>,
}
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
enum Output {
    Projection(Projection),
    Registry(Registry),
    Membership(Membership),
    Subsets(Subsets),
}

fn run(input: &Input) -> Result<Output, OwnedParameterNamesError> {
    match input {
        Input::WireNameProjection { name, owner, root } => Ok(Output::Projection(Projection {
            wire: wire(name),
            names: wire_names(*owner)?,
            owned: root.map(owned_wire_names).transpose()?,
        })),
        Input::OwnedNameRegistry { names } => Ok(Output::Registry(Registry {
            names: owned_names()?,
            owned: names
                .iter()
                .map(|name| is_litellm_owned_kwarg(name))
                .collect::<Result<Vec<_>, _>>()?,
        })),
        Input::InternalPrefixMembership { names } => Ok(Output::Membership(Membership {
            owned: names
                .iter()
                .map(|name| is_litellm_owned_kwarg(name))
                .collect::<Result<Vec<_>, _>>()?,
        })),
        Input::SubsetRegistries => {
            let mut agentic_loop = wire_names(Owner::AgenticLoopState)?;
            agentic_loop.extend(wire_names(Owner::AgenticLoopOptions)?);
            Ok(Output::Subsets(Subsets {
                agentic_loop,
                bedrock_batch: wire_names(Owner::BedrockBatchConnection)?,
            }))
        }
    }
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let output = run(input).map_err(|error| {
            Box::new(HelperError {
                class: "TypeError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })?;
        serde_json::to_value(output).map_err(|error| {
            Box::new(HelperError {
                class: "SerializationError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })
    }
}
