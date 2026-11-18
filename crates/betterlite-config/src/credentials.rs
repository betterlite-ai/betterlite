use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::resource_rows::ResourceRowsError;
use betterlite_db::resource_rows::credential;

// the config's credential_list entries are the same model the registry and the database rows use
pub type CredentialItem = credential::CredentialItem;

#[derive(Debug, thiserror::Error)]
pub enum CredentialListError {
    #[error("credential_list must be a list of credential objects")]
    NotAList,
    #[error("credential_list[{index}]: {source}")]
    Entry {
        index: usize,
        #[source]
        source: ResourceRowsError,
    },
}

pub fn parse_credential_list(
    value: Option<JsonData>,
) -> Result<Vec<CredentialItem>, CredentialListError> {
    match value {
        None | Some(JsonData::Null) => Ok(Vec::new()),
        Some(JsonData::Array(entries)) => {
            let mut credentials = Vec::with_capacity(entries.len());
            for (index, entry) in entries.into_iter().enumerate() {
                let credential = CredentialItem::try_from(entry)
                    .map_err(|source| CredentialListError::Entry { index, source })?;
                credentials.push(credential);
            }
            Ok(credentials)
        }
        Some(_) => Err(CredentialListError::NotAList),
    }
}

mod tests;
