use crate::database_tokens::endpoint::IAMEndpoint;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriterEndpoint {
    pub host: Option<String>,
    pub port: String,
    pub user: Option<String>,
    pub name: Option<String>,
    pub schema: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum WriterEndpointError {
    #[error(
        "Cannot mint a database token: {missing} unset. Set them so the connection URL can be reassembled around a freshly minted token."
    )]
    MissingFields { missing: String },
}

fn missing_names(host: Option<&str>, user: Option<&str>, name: Option<&str>) -> Vec<&'static str> {
    [
        ("DATABASE_HOST", host),
        ("DATABASE_USER", user),
        ("DATABASE_NAME", name),
    ]
    .into_iter()
    .filter(|(_, value)| value.is_none())
    .map(|(field, _)| field)
    .collect()
}

impl WriterEndpoint {
    pub fn to_iam_endpoint(&self) -> Result<IAMEndpoint, WriterEndpointError> {
        let host = self.host.as_deref().filter(|value| !value.is_empty());
        let user = self.user.as_deref().filter(|value| !value.is_empty());
        let name = self.name.as_deref().filter(|value| !value.is_empty());
        let (Some(host_value), Some(user_value), Some(name_value)) = (host, user, name) else {
            return Err(WriterEndpointError::MissingFields {
                missing: missing_names(host, user, name).join(", "),
            });
        };
        Ok(IAMEndpoint {
            host: host_value.to_owned(),
            port: self.port.clone(),
            user: user_value.to_owned(),
            name: name_value.to_owned(),
            schema: self.schema.clone(),
        })
    }
}
