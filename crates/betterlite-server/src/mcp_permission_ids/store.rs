use betterlite_db::auxiliary_repository::repository::{PrismaClient, TableError};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::resource_repository::permission::ObjectPermissionRepository;
use betterlite_db::resource_repository::table::KeyFilter;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ExistingServersError {
    #[error(transparent)]
    Database(#[from] TableError),
    #[error("the stored mcp_servers is not a list of server identifier strings")]
    ServersNotAList,
}

// ast-grep-ignore: no-long-names - the LiteLLM function this task owns
pub async fn existing_object_permission_mcp_servers(
    object_permission_id: Option<&str>,
    client: Option<PrismaClient<'_>>,
) -> Result<Vec<String>, ExistingServersError> {
    let Some(pool) = client else {
        return Ok(Vec::new());
    };
    let Some(id) = object_permission_id else {
        return Ok(Vec::new());
    };
    if id.is_empty() {
        return Ok(Vec::new());
    }
    let table = ObjectPermissionRepository::new(Some(pool), false)
        .table()
        .map_err(TableError::Database)?;
    let rows = table
        .as_ref()
        .find::<JsonData>(KeyFilter::Equal(id))
        .await?;
    let Some(row) = rows.into_iter().next() else {
        return Ok(Vec::new());
    };
    let JsonData::Object(fields) = row else {
        return Err(ExistingServersError::ServersNotAList);
    };
    server_identifiers(fields.get("mcp_servers"))
}

fn server_identifiers(field: Option<&JsonData>) -> Result<Vec<String>, ExistingServersError> {
    match field {
        None | Some(JsonData::Null) => Ok(Vec::new()),
        Some(JsonData::Array(items)) => {
            let mut servers = Vec::new();
            for item in items {
                let JsonData::String(server) = item else {
                    return Err(ExistingServersError::ServersNotAList);
                };
                servers.push(server.clone());
            }
            Ok(servers)
        }
        Some(
            JsonData::Bool(_) | JsonData::Number(_) | JsonData::String(_) | JsonData::Object(_),
        ) => Err(ExistingServersError::ServersNotAList),
    }
}
