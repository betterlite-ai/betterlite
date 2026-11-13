use serde::Serialize;
use url::form_urlencoded::Serializer;

use crate::zerobus::client::error::ZerobusClientError;
use crate::zerobus::client::types::ZerobusConnection;

pub const TOKEN_PATH: &str = "/oidc/v1/token";
pub const OAUTH_SCOPE: &str = "all-apis";

#[derive(Serialize)]
struct Privilege {
    #[serde(rename = "type")]
    kind: &'static str,
    privileges: &'static [&'static str],
    object_type: &'static str,
    object_full_path: String,
}

// The token endpoint refuses a token that does not carry the table's Unity Catalog privileges
fn authorization_details(table_name: &str) -> Result<String, ZerobusClientError> {
    let mut parts = table_name.splitn(3, '.');
    let (Some(catalog), Some(schema), Some(_table)) = (parts.next(), parts.next(), parts.next())
    else {
        return Err(ZerobusClientError::TableName {
            table_name: table_name.to_owned(),
        });
    };
    let details = vec![
        Privilege {
            kind: "unity_catalog_privileges",
            privileges: &["USE CATALOG"],
            object_type: "CATALOG",
            object_full_path: catalog.to_owned(),
        },
        Privilege {
            kind: "unity_catalog_privileges",
            privileges: &["USE SCHEMA"],
            object_type: "SCHEMA",
            object_full_path: format!("{catalog}.{schema}"),
        },
        Privilege {
            kind: "unity_catalog_privileges",
            privileges: &["SELECT", "MODIFY"],
            object_type: "TABLE",
            object_full_path: table_name.to_owned(),
        },
    ];
    serde_json::to_string(&details).map_err(ZerobusClientError::AuthorizationDetails)
}

pub fn token_form(connection: &ZerobusConnection) -> Result<String, ZerobusClientError> {
    Ok(Serializer::new(String::new())
        .append_pair("grant_type", "client_credentials")
        .append_pair("scope", OAUTH_SCOPE)
        .append_pair(
            "resource",
            &format!(
                "api://databricks/workspaces/{}/zerobusDirectWriteApi",
                connection.workspace_id
            ),
        )
        .append_pair(
            "authorization_details",
            &authorization_details(&connection.table_name)?,
        )
        .finish())
}
