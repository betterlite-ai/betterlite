#![cfg(test)]

use super::support::caller_headers;
use super::{GetAgentRequest, ListAgentsRequest, Params};

pub(crate) fn list_request(litellm_params: &Params) -> ListAgentsRequest<'_> {
    ListAgentsRequest {
        litellm_params,
        extra_headers: caller_headers(&[]),
        timeout: None,
    }
}

pub(crate) fn get_request<'call>(
    name: &'call str,
    litellm_params: &'call Params,
) -> GetAgentRequest<'call> {
    GetAgentRequest {
        name,
        litellm_params,
        extra_headers: caller_headers(&[]),
        timeout: None,
    }
}
