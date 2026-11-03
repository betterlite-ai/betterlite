pub(crate) struct Op;

#[derive(serde::Deserialize)]
pub(crate) struct Input {
    principal: Option<betterlite_core::api::managed_resource_store::OwnerPrincipal>,
    owner: Option<String>,
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let principal = input.principal.as_ref();
        Ok(serde_json::json!({
            "is_proxy_admin": betterlite_core::api::managed_resource_store::is_proxy_admin(principal),
            "scopes": betterlite_core::api::managed_resource_store::get_resource_owner_scopes(principal),
            "primary": betterlite_core::api::managed_resource_store::get_primary_owner_scope(principal),
            "access": betterlite_core::api::managed_resource_store::user_can_access_resource_owner(input.owner.as_deref(), principal),
        }))
    }
}
