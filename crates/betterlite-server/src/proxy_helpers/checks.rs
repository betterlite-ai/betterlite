use std::collections::BTreeMap;
use std::error::Error;

use axum::http::StatusCode;
use betterlite_models::decisions::Json;

pub const NOT_PREMIUM_USER: &str = "This proxy build does not include the enterprise feature set.";

#[derive(Debug, Clone)]
pub enum Detail {
    Text(String),
    Map(BTreeMap<String, Json>),
}

impl Detail {
    fn text(&self) -> String {
        match self {
            Self::Text(text) => text.clone(),
            Self::Map(map) => serde_json::to_string(map).unwrap_or_default(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct HttpException {
    pub status: StatusCode,
    pub detail: Option<Detail>,
    pub message: Option<Detail>,
}

#[derive(Clone, Copy)]
pub enum ErrorText<'error> {
    Http(&'error HttpException),
    Other(&'error dyn Error),
}

#[must_use]
pub fn error_message_str(source: ErrorText<'_>) -> String {
    match source {
        // both absent leaves only Python's repr of the detail, which betterlite does not carry
        ErrorText::Http(http) => http
            .detail
            .as_ref()
            .or(http.message.as_ref())
            .map_or_else(String::new, Detail::text),
        ErrorText::Other(error) => error.to_string(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProxyCheckError {
    #[error("Invalid model for team {team_id}: {model}.  Valid models for team are: {models}\n")]
    InvalidTeamModel {
        team_id: String,
        model: String,
        models: String,
    },
    #[error("the request data carries no model name")]
    MissingRequestModel,
    #[error("the team config models must be a list of model names")]
    InvalidTeamModels,
    #[error("{message}")]
    PremiumUser { message: String },
    #[error("{message}")]
    Database { message: String },
}

impl ProxyCheckError {
    #[must_use]
    pub const fn status_code(&self) -> StatusCode {
        match self {
            Self::PremiumUser { .. } => StatusCode::FORBIDDEN,
            Self::InvalidTeamModel { .. }
            | Self::MissingRequestModel
            | Self::InvalidTeamModels
            | Self::Database { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    // the HTTPException detail LiteLLM raises: {"error": message}
    #[must_use]
    pub fn detail(&self) -> Option<BTreeMap<String, Json>> {
        match self {
            Self::PremiumUser { message } | Self::Database { message } => Some(BTreeMap::from([(
                "error".to_owned(),
                Json::Text(message.clone()),
            )])),
            Self::InvalidTeamModel { .. } | Self::MissingRequestModel | Self::InvalidTeamModels => {
                None
            }
        }
    }
}

pub fn check_team_models(
    team_id: Option<&str>,
    team_config: Option<&mut BTreeMap<String, Json>>,
    request_data: Option<&BTreeMap<String, Json>>,
) -> Result<(), ProxyCheckError> {
    let (Some(id), Some(config), Some(request)) = (team_id, team_config, request_data) else {
        return Ok(());
    };
    let Some(models) = config.remove("models") else {
        return Ok(());
    };
    let Json::List(items) = models else {
        return Err(ProxyCheckError::InvalidTeamModels);
    };
    let mut names = Vec::with_capacity(items.len());
    for item in &items {
        let Json::Text(name) = item else {
            return Err(ProxyCheckError::InvalidTeamModels);
        };
        names.push(name.clone());
    }
    let Some(Json::Text(model)) = request.get("model") else {
        return Err(ProxyCheckError::MissingRequestModel);
    };
    if names.iter().any(|name| name == model) {
        return Ok(());
    }
    Err(ProxyCheckError::InvalidTeamModel {
        team_id: id.to_owned(),
        model: model.clone(),
        models: names.join(", "),
    })
}

pub fn premium_user_check(
    premium_user: bool,
    feature: Option<&str>,
) -> Result<(), ProxyCheckError> {
    if premium_user {
        return Ok(());
    }
    let message = feature.map_or_else(
        || format!("This feature is only available for enterprise users. {NOT_PREMIUM_USER}"),
        |name| {
            format!(
                "This feature is only available for enterprise users: {name}. {NOT_PREMIUM_USER}"
            )
        },
    );
    Err(ProxyCheckError::PremiumUser { message })
}

pub fn prisma_client_or_throw<'database, Client>(
    database: Option<&'database Client>,
    message: &str,
) -> Result<&'database Client, ProxyCheckError> {
    database.ok_or_else(|| ProxyCheckError::Database {
        message: message.to_owned(),
    })
}
