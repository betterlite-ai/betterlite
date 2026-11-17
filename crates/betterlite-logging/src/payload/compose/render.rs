use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::error_info;
use crate::fields::payload::records::{ErrorInformation, StandardLoggingHiddenParams};
use crate::payload::compose::costs;
use crate::payload::compose::proxy_request;
use crate::payload::compose::response_messages;
use crate::payload::compose::{Basis, PayloadComposition, PayloadRequest};
use crate::payload::error::PayloadError;
use crate::spend_bodies::redaction::should_redact_message_logging;

// The rendered response and costs, and the fields that pass straight through.
pub(super) struct Render {
    pub(super) response: Json,
    pub(super) model: String,
    pub(super) costs: costs::Costs,
    pub(super) hidden_params: StandardLoggingHiddenParams,
    pub(super) error_information: ErrorInformation,
    pub(super) error_str: Option<String>,
    pub(super) messages: Option<Json>,
    pub(super) model_parameters: IndexMap<String, Json>,
    pub(super) request_tags: Vec<Json>,
    pub(super) end_user: Option<String>,
    pub(super) requester_ip_address: Option<String>,
    pub(super) user_agent: Option<String>,
}

impl Render {
    pub(super) fn read(
        request: &PayloadRequest<'_>,
        composition: &PayloadComposition<'_>,
        basis: &Basis,
    ) -> Result<Self, PayloadError> {
        let redact = should_redact_message_logging(
            Some(&request.state.litellm_params),
            Some(&request.state.callback_dynamic_params),
            composition.privacy.turn_off_message_logging,
        );
        let response = response_messages::final_response(request, &basis.extracted, redact);
        let model = response_messages::model_name(
            request,
            &basis.extracted,
            &basis.identifiers.model,
            &response,
        );
        let (costs, hidden_params) = priced(request, composition, basis, &model)?;
        let (error_information, error_str) =
            error_info::build(request, &basis.metadata, composition)?;
        let messages = response_messages::logged_messages(request, composition, redact)?;
        let model_parameters = response_messages::model_parameters(request.state)?;
        let request_tags = proxy_request::request_tags(&basis.params, &basis.proxy, composition)?;
        Ok(Self {
            end_user: basis
                .clean_metadata
                .user_api_key_end_user_id
                .clone()
                .or_else(|| basis.proxy.body_user.clone()),
            requester_ip_address: basis.clean_metadata.requester_ip_address.clone(),
            user_agent: basis.clean_metadata.user_agent.clone(),
            response,
            model,
            costs,
            hidden_params,
            error_information,
            error_str,
            messages,
            model_parameters,
            request_tags,
        })
    }
}

fn priced(
    request: &PayloadRequest<'_>,
    composition: &PayloadComposition<'_>,
    basis: &Basis,
    model: &str,
) -> Result<(costs::Costs, StandardLoggingHiddenParams), PayloadError> {
    let costs = costs::Costs::read(
        &costs::CostsInput {
            request,
            metadata: &basis.metadata,
            usage: &basis.usage,
            identifiers: &basis.identifiers,
            model,
        },
        composition,
    )?;
    let hidden_params = costs::hidden_params(request, &basis.extracted, costs.raw_response_cost)?;
    Ok((costs, hidden_params))
}
