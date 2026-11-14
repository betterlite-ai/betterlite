use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use thiserror::Error;

use crate::scim_projections::directory::Metadata;
use crate::scim_records::attribute::DirectoryAttribute;
use crate::scim_records::user::{EnterpriseUser, Manager};
use crate::scim_records::{ENTERPRISE_METADATA_KEY, ENTITLEMENTS_METADATA_KEY, ROLES_METADATA_KEY};

use super::data::ScimUserData;

const SCIM_METADATA_KEY: &str = "scim_metadata";
const SCIM_ACTIVE_KEY: &str = "scim_active";
const MANAGER_REFERENCE_KEY: &str = "$ref";

#[derive(Debug, Error)]
pub enum MetadataError {
    #[error("a SCIM directory attribute is not valid JSON")]
    Attribute(#[source] serde_json::Error),
}

// LiteLLM always writes scim_metadata, adds scim_active only when supplied, and dumps each optional block without None fields
pub fn build_scim_metadata(
    data: &ScimUserData,
    active: Option<bool>,
) -> Result<Metadata, MetadataError> {
    let mut metadata = Metadata::new();
    metadata.insert(SCIM_METADATA_KEY.to_owned(), name_block(data));
    if let Some(flag) = active {
        metadata.insert(SCIM_ACTIVE_KEY.to_owned(), JsonData::Bool(flag));
    }
    if let Some(enterprise) = data.enterprise.as_ref() {
        metadata.insert(
            ENTERPRISE_METADATA_KEY.to_owned(),
            enterprise_block(enterprise),
        );
    }
    if let Some(entitlements) = data.entitlements.as_deref() {
        metadata.insert(
            ENTITLEMENTS_METADATA_KEY.to_owned(),
            attribute_list(entitlements)?,
        );
    }
    if let Some(roles) = data.roles.as_deref() {
        metadata.insert(ROLES_METADATA_KEY.to_owned(), attribute_list(roles)?);
    }
    Ok(metadata)
}

// LiteLLM_UserScimMetadata.model_dump() keeps both camelCase keys, null when unset
fn name_block(data: &ScimUserData) -> JsonData {
    let mut fields = BTreeMap::new();
    fields.insert(
        "givenName".to_owned(),
        text_field(data.given_name.as_deref()),
    );
    fields.insert(
        "familyName".to_owned(),
        text_field(data.family_name.as_deref()),
    );
    JsonData::Object(fields)
}

fn text_field(value: Option<&str>) -> JsonData {
    let Some(text) = value else {
        return JsonData::Null;
    };
    JsonData::String(text.to_owned())
}

fn enterprise_block(enterprise: &EnterpriseUser) -> JsonData {
    let mut fields = BTreeMap::new();
    insert_text(
        &mut fields,
        "employeeNumber",
        enterprise.employee_number.as_deref(),
    );
    insert_text(&mut fields, "costCenter", enterprise.cost_center.as_deref());
    insert_text(
        &mut fields,
        "organization",
        enterprise.organization.as_deref(),
    );
    insert_text(&mut fields, "division", enterprise.division.as_deref());
    insert_text(&mut fields, "department", enterprise.department.as_deref());
    if let Some(manager) = enterprise.manager.as_ref() {
        fields.insert("manager".to_owned(), manager_block(manager));
    }
    JsonData::Object(fields)
}

// SCIMUserManager dumps by alias, so the reference column keeps its $ref name
fn manager_block(manager: &Manager) -> JsonData {
    let mut fields = BTreeMap::new();
    insert_text(&mut fields, "value", manager.value.as_deref());
    insert_text(&mut fields, "displayName", manager.display_name.as_deref());
    insert_text(
        &mut fields,
        MANAGER_REFERENCE_KEY,
        manager.reference.as_deref(),
    );
    JsonData::Object(fields)
}

fn insert_text(fields: &mut BTreeMap<String, JsonData>, key: &str, value: Option<&str>) {
    if let Some(text) = value {
        fields.insert(key.to_owned(), JsonData::String(text.to_owned()));
    }
}

fn attribute_list(attributes: &[DirectoryAttribute]) -> Result<JsonData, MetadataError> {
    let mut dumped = Vec::with_capacity(attributes.len());
    for attribute in attributes {
        dumped.push(attribute_block(attribute)?);
    }
    Ok(JsonData::Array(dumped))
}

// SCIMMultiValuedAttribute allows extras, and model_dump(exclude_none=True) drops every None
fn attribute_block(attribute: &DirectoryAttribute) -> Result<JsonData, MetadataError> {
    let mut fields = BTreeMap::new();
    insert_text(&mut fields, "value", attribute.fields.value.as_deref());
    insert_text(&mut fields, "display", attribute.fields.display.as_deref());
    insert_text(&mut fields, "type", attribute.fields.kind.as_deref());
    if let Some(primary) = attribute.fields.primary {
        fields.insert("primary".to_owned(), JsonData::Bool(primary.0));
    }
    for (key, raw) in &attribute.extra.0 {
        let value =
            serde_json::from_str::<JsonData>(raw.get()).map_err(MetadataError::Attribute)?;
        if !matches!(value, JsonData::Null) {
            fields.insert(key.clone(), value);
        }
    }
    Ok(JsonData::Object(fields))
}
