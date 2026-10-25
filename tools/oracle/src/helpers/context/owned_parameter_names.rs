use crate::helpers::HelperError;
use betterlite_context::owned_parameter_names::{
    AGENTIC_LOOP_KWARG_NAMES, BEDROCK_BATCH_KWARG_NAMES, OwnedParameterNamesError, Owner,
    WireMetadata, is_litellm_owned_kwarg, owned_names, owned_wire_names, wire_names,
};
use serde::{Deserialize, Serialize};

mod tests;

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    WireNameProjection {
        name: String,
        owner: Owner,
        root: Option<Owner>,
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
#[serde(untagged)]
enum Output<'input> {
    Projection {
        wire: WireMetadata<'input>,
        names: Vec<String>,
        owned: Option<Vec<String>>,
    },
    Registry {
        names: Vec<String>,
        owned: Vec<bool>,
    },
    Membership {
        owned: Vec<bool>,
    },
    Subsets {
        agentic_loop: Vec<String>,
        bedrock_batch: Vec<String>,
    },
}

fn run(input: &Input) -> Result<Output<'_>, OwnedParameterNamesError> {
    match input {
        Input::WireNameProjection { name, owner, root } => Ok(Output::Projection {
            wire: WireMetadata { wire_name: name },
            names: wire_names(*owner),
            owned: root
                .map(|root_owner| owned_wire_names(root_owner.record()))
                .transpose()?,
        }),
        Input::OwnedNameRegistry { names } => Ok(Output::Registry {
            names: owned_names(),
            owned: names
                .iter()
                .map(|name| is_litellm_owned_kwarg(name))
                .collect(),
        }),
        Input::InternalPrefixMembership { names } => Ok(Output::Membership {
            owned: names
                .iter()
                .map(|name| is_litellm_owned_kwarg(name))
                .collect(),
        }),
        Input::SubsetRegistries => Ok(Output::Subsets {
            agentic_loop: AGENTIC_LOOP_KWARG_NAMES.names(),
            bedrock_batch: BEDROCK_BATCH_KWARG_NAMES.names(),
        }),
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
