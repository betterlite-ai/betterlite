use serde::Serialize;

schema_list! { ListSchemas, "urn:ietf:params:scim:api:messages:2.0:ListResponse" }
// LiteLLM lists only users or only groups, so Resource is User or Group
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListResponse<Resource> {
    pub schemas: ListSchemas,
    pub total_results: u64,
    pub start_index: u64,
    pub items_per_page: u64,
    #[serde(rename = "Resources")]
    pub resources: Vec<Resource>,
}
