use betterlite_db::resource_rows::access_group::AccessGroupTable;
use betterlite_db::resource_rows::config::ConfigRow;
use betterlite_db::resource_rows::credential::{
    CreateCredentialItem, CredentialBase, CredentialItem, UpdateCredentialItem,
};
use betterlite_db::resource_rows::model::ProxyModelTable;
use betterlite_db::resource_rows::organization::OrganizationTable;
use betterlite_db::resource_rows::project::ProjectTable;

use super::budget_permission_rows::Validated;

pub(crate) struct Op;

#[derive(serde::Deserialize)]
#[serde(tag = "operation")]
pub(crate) enum Input {
    #[serde(rename = "access_group")]
    AccessGroup { row: Validated<AccessGroupTable> },
    #[serde(rename = "config")]
    Config { row: Validated<ConfigRow> },
    #[serde(rename = "credential_base")]
    CredentialBase { row: Validated<CredentialBase> },
    #[serde(rename = "credential_item")]
    CredentialItem { row: Validated<CredentialItem> },
    #[serde(rename = "credential_create")]
    CredentialCreate {
        row: Validated<CreateCredentialItem>,
    },
    #[serde(rename = "credential_update")]
    CredentialUpdate {
        row: Validated<UpdateCredentialItem>,
    },
    #[serde(rename = "model")]
    Model { row: Validated<ProxyModelTable> },
    #[serde(rename = "organization")]
    Organization { row: Validated<OrganizationTable> },
    #[serde(rename = "project")]
    Project { row: Validated<ProjectTable> },
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let value = match input {
            Input::AccessGroup { row } => serde_json::to_value(validated(row)?),
            Input::Config { row } => serde_json::to_value(validated(row)?),
            Input::CredentialBase { row } => serde_json::to_value(validated(row)?),
            Input::CredentialItem { row } => serde_json::to_value(validated(row)?),
            Input::CredentialCreate { row } => serde_json::to_value(validated(row)?),
            Input::CredentialUpdate { row } => serde_json::to_value(validated(row)?),
            Input::Model { row } => serde_json::to_value(validated(row)?),
            Input::Organization { row } => serde_json::to_value(validated(row)?),
            Input::Project { row } => serde_json::to_value(validated(row)?),
        };
        value.map_err(|error| helper_error("InvalidOutput", &error.to_string()))
    }
}

fn validated<T>(row: &Validated<T>) -> Result<&T, Box<crate::helpers::HelperError>> {
    match row {
        Validated::Valid(value) => Ok(value),
        Validated::Invalid(_) => Err(helper_error("ValidationError", "invalid row")),
    }
}

fn helper_error(class: &str, message: &str) -> Box<crate::helpers::HelperError> {
    let error = crate::helpers::HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    };
    Box::new(error)
}
