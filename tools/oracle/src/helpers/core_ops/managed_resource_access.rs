use betterlite_core::api::managed_resource_access::{
    AuthenticatedPrincipal, ResourceRecord, build_list_page, build_owner_filter,
    can_access_resource, resolve_resource_owner_id,
};
use serde::Deserialize;
use serde_json::json;

pub(crate) struct Op;

#[derive(Deserialize)]
pub struct Input {
    principal: AuthenticatedPrincipal,
    #[serde(default)]
    records: Vec<ResourceRecord>,
    #[serde(default)]
    has_more: bool,
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let owner_filter = match build_owner_filter(&input.principal) {
            None => serde_json::Value::Null,
            Some(filter) => match filter {
                betterlite_core::api::managed_resource_access::OwnerFilter::CreatedBy {
                    created_by,
                } => json!({"created_by": created_by}),
                betterlite_core::api::managed_resource_access::OwnerFilter::TeamId { team_id } => {
                    json!({"team_id": team_id})
                }
                betterlite_core::api::managed_resource_access::OwnerFilter::UserOrTeam {
                    user_id,
                    team_id,
                } => json!({"OR": [{"created_by": user_id}, {"team_id": team_id}]}),
                betterlite_core::api::managed_resource_access::OwnerFilter::Admin => json!({}),
            },
        };
        let access = input
            .records
            .iter()
            .map(|record| {
                can_access_resource(
                    &input.principal,
                    record.created_by.as_deref(),
                    record.team_id.as_deref(),
                )
            })
            .collect::<Vec<_>>();
        Ok(json!({
            "owner": resolve_resource_owner_id(&input.principal),
            "filter": owner_filter,
            "access": access,
            "page": build_list_page(input.records.clone(), input.has_more),
        }))
    }
}
