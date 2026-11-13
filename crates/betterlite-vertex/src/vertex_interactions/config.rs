use std::sync::Arc;

use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};

use crate::credential_sources::getters::VertexParams;
use crate::endpoint_target::target::get_api_base;
use crate::model_urls::base_url::validate_vertex_location;
use crate::studio_interactions::credentials::RequestHeaders;
use crate::studio_interactions::urls::Lifecycle;
use crate::token_cache::probe::AccessToken;

use super::DEFAULT_LOCATION;
use super::error::VertexInteractionsError;
use super::mint::MintAccessToken;
use super::request::{EmptyBody, LifecycleRequest};
use super::target::VertexInteractionsTarget;

#[derive(Debug, Clone, Copy)]
pub struct InteractionCall<'call> {
    pub api_base: Option<&'call str>,
    pub params: VertexParams<'call>,
    pub settings: &'call VertexSettings,
}

pub struct VertexInteractionsConfig {
    pub mint: Arc<dyn MintAccessToken>,
}

impl VertexInteractionsConfig {
    async fn mint_for(
        &self,
        call: &InteractionCall<'_>,
    ) -> Result<AccessToken, VertexInteractionsError> {
        self.mint
            .mint(
                call.params.safe_get_vertex_ai_credentials(call.settings),
                call.params.safe_get_vertex_ai_project(call.settings),
            )
            .await
            .map_err(VertexInteractionsError::Mint)
    }

    // every target mints, requires a resolved project, and takes the explicit location or global
    pub async fn target(
        &self,
        call: &InteractionCall<'_>,
    ) -> Result<VertexInteractionsTarget, VertexInteractionsError> {
        let access = self.mint_for(call).await?;
        if access.project.is_empty() {
            return Err(VertexInteractionsError::MissingProject);
        }
        let location = validate_vertex_location(Some(
            call.params
                .explicit_vertex_ai_location()
                .unwrap_or(DEFAULT_LOCATION),
        ))
        .map_err(VertexInteractionsError::Location)?;
        let base_url = get_api_base(call.api_base, Some(location))
            .map_err(VertexInteractionsError::ApiBase)?;
        Ok(VertexInteractionsTarget {
            base_url,
            project_id: access.project,
            location: location.to_owned(),
        })
    }

    pub async fn get_complete_url(
        &self,
        call: &InteractionCall<'_>,
        stream: bool,
    ) -> Result<String, VertexInteractionsError> {
        let collection_url = self.target(call).await?.collection_url();
        if stream {
            return Ok(format!("{collection_url}?alt=sse"));
        }
        Ok(collection_url)
    }

    // the Vertex override mints again here and only adds JSON and Bearer, so caller headers win over both
    pub async fn headers(
        &self,
        call: &InteractionCall<'_>,
        caller: &IndexMap<String, String>,
    ) -> Result<RequestHeaders, VertexInteractionsError> {
        let access = self.mint_for(call).await?;
        let mut headers: RequestHeaders = IndexMap::with_capacity(caller.len().saturating_add(2));
        headers.insert(
            "Content-Type".to_owned(),
            SecretString::from("application/json"),
        );
        headers.insert(
            "Authorization".to_owned(),
            SecretString::from(format!("Bearer {}", access.token.expose_secret())),
        );
        for (name, value) in caller {
            headers.insert(name.clone(), SecretString::from(value.as_str()));
        }
        Ok(headers)
    }

    // get and delete carry no suffix, cancel appends one outside the encoded id, and every body is empty
    pub async fn lifecycle_request(
        &self,
        call: &InteractionCall<'_>,
        action: Lifecycle,
        interaction_id: &str,
    ) -> Result<LifecycleRequest, VertexInteractionsError> {
        let interaction_url = self.target(call).await?.interaction_url(interaction_id)?;
        let suffix = match action {
            Lifecycle::Get | Lifecycle::Delete => "",
            Lifecycle::Cancel => ":cancel",
        };
        Ok(LifecycleRequest {
            url: format!("{interaction_url}{suffix}"),
            body: EmptyBody::default(),
        })
    }
}
