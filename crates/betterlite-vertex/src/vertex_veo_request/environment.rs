use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_openai::vector_stores::Headers;
use secrecy::{ExposeSecret as _, SecretString};

use crate::credential_sources::getters::VertexParams;
use crate::vertex_interactions::mint::MintAccessToken;

use super::error::VeoRequestError;

// validate_environment: Bearer and JSON defaults first, then every caller header overrides them
pub async fn validate_environment(
    caller: Headers,
    params: VertexParams<'_>,
    settings: &VertexSettings,
    mint: &dyn MintAccessToken,
) -> Result<Headers, VeoRequestError> {
    let access = mint
        .mint(
            params.safe_get_vertex_ai_credentials(settings),
            params.safe_get_vertex_ai_project(settings),
        )
        .await
        .map_err(VeoRequestError::Mint)?;
    let mut headers = Headers::new();
    headers.insert(
        "Authorization".to_owned(),
        SecretString::from(format!("Bearer {}", access.token.expose_secret())),
    );
    headers.insert(
        "Content-Type".to_owned(),
        SecretString::from("application/json"),
    );
    for (name, value) in caller {
        headers.insert(name, value);
    }
    Ok(headers)
}
