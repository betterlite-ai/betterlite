use betterlite_auth::management_routes;

#[derive(serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum Group {
    KeyEnum,
    MasterKeyOnlyRoutes,
    KeyManagementRoutes,
    TeamServiceAccountKeyRoutes,
    ManagementRoutes,
    SpendTrackingRoutes,
    GlobalSpendTrackingRoutes,
    InternalUserRoutes,
    InternalUserViewOnlyRoutes,
    OrgAdminOnlyRoutes,
    AdminViewerRoutes,
    OrgAdminAllowedRoutes,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    group: Group,
}

pub(crate) struct Op;

fn values(group: &Group) -> Vec<&'static str> {
    match group {
        Group::KeyEnum => key_values(),
        Group::MasterKeyOnlyRoutes => management_routes::MASTER_KEY_ONLY_ROUTES.to_vec(),
        Group::KeyManagementRoutes => management_routes::KEY_MANAGEMENT_ROUTES.to_vec(),
        Group::TeamServiceAccountKeyRoutes => {
            management_routes::TEAM_SERVICE_ACCOUNT_ROUTES.to_vec()
        }
        Group::ManagementRoutes => management_routes::management_routes(),
        Group::SpendTrackingRoutes => management_routes::SPEND_TRACKING_ROUTES.to_vec(),
        Group::GlobalSpendTrackingRoutes => {
            management_routes::GLOBAL_SPEND_TRACKING_ROUTES.to_vec()
        }
        Group::InternalUserRoutes => management_routes::internal_user_routes(),
        Group::InternalUserViewOnlyRoutes => {
            management_routes::INTERNAL_USER_VIEW_ONLY_ROUTES.to_vec()
        }
        Group::OrgAdminOnlyRoutes => management_routes::ORG_ADMIN_ONLY_ROUTES.to_vec(),
        Group::AdminViewerRoutes => management_routes::admin_viewer_routes(),
        Group::OrgAdminAllowedRoutes => management_routes::org_admin_allowed_routes(),
    }
}

fn key_values() -> Vec<&'static str> {
    [
            management_routes::KeyManagementRoute::KeyGenerate,
            management_routes::KeyManagementRoute::KeyUpdate,
            management_routes::KeyManagementRoute::KeyDelete,
            management_routes::KeyManagementRoute::KeyRegenerate,
            management_routes::KeyManagementRoute::KeyGenerateServiceAccount,
            management_routes::KeyManagementRoute::KeyRegenerateWithPathParam,
            management_routes::KeyManagementRoute::KeyBlock,
            management_routes::KeyManagementRoute::KeyUnblock,
            management_routes::KeyManagementRoute::KeyBulkUpdate,
            management_routes::KeyManagementRoute::TeamKeyBulkUpdate,
            management_routes::KeyManagementRoute::KeyResetSpend,
            management_routes::KeyManagementRoute::KeyAccessGroupAssignment,
            management_routes::KeyManagementRoute::AutoRouterManage,
            management_routes::KeyManagementRoute::KeyInfo,
            management_routes::KeyManagementRoute::KeyHealth,
            management_routes::KeyManagementRoute::KeyList,
            management_routes::KeyManagementRoute::KeyAliases,
            management_routes::KeyManagementRoute::TeamDailyActivity,
            management_routes::KeyManagementRoute::TeamDailyActivityAggregated,
            management_routes::KeyManagementRoute::SpendLogs,
            management_routes::KeyManagementRoute::SpendLogsV2,
        ]
        .iter()
        .map(|route| route.value())
    .iter()
    .map(|route| route.value())
    .collect()
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        serde_json::to_value(values(&input.group)).map_err(|error| {
            Box::new(crate::helpers::HelperError {
                class: "SerializationError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })
    }
}
