#![cfg(test)]

macro_rules! proxy_input {
    ($api_base:expr, $stream:expr) => {
        crate::endpoint_target::target::CustomProxyInput {
            api_base: $api_base,
            provider: crate::endpoint_target::target::TargetProvider::Vertex,
            gemini_api_key: None,
            endpoint: "generateContent",
            stream: $stream,
            auth_header: None,
            url: super::support::NATIVE_URL,
            model: Some(super::support::MODEL),
            project: Some(super::support::PROJECT),
            location: Some(super::support::LOCATION),
            api_version: crate::model_urls::ApiVersion::V1,
            use_psc_endpoint_format: false,
        }
    };
}

macro_rules! token_input {
    ($auth_token:expr) => {
        crate::endpoint_target::target::TokenUrlInput {
            provider: crate::endpoint_target::target::TargetProvider::Vertex,
            model: super::support::MODEL,
            auth_token: $auth_token,
            gemini_api_key: None,
            project: Some(super::support::PROJECT),
            location: Some(super::support::LOCATION),
            stream: false,
            api_base: None,
            api_version: crate::model_urls::ApiVersion::V1,
            mode: crate::model_urls::UrlMode::Chat,
            use_psc_endpoint_format: false,
        }
    };
}

macro_rules! partner_request {
    ($partner:expr, $stream:expr, $custom_api_base:expr) => {
        crate::endpoint_target::target::PartnerRequest {
            custom_api_base: $custom_api_base,
            location: Some(super::support::LOCATION),
            vertex_project: Some(super::support::PROJECT),
            project_id: "defproj",
            partner: $partner,
            stream: $stream,
            model: super::support::MODEL,
        }
    };
}

macro_rules! partner_input {
    ($partner:expr, $stream:expr, $api_base:expr) => {
        crate::endpoint_target::partner::PartnerUrlInput {
            location: super::support::LOCATION,
            project: super::support::PROJECT,
            partner: $partner,
            stream: $stream,
            model: super::support::MODEL,
            api_base: $api_base,
        }
    };
}

mod graft;
mod headers;
mod partner;
mod passthrough;
mod proxy;
mod region;
mod support;
mod token;
