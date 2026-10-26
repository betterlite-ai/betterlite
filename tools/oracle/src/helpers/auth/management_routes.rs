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

#[derive(serde::Serialize)]
#[serde(untagged)]
enum Output {
    Keys(
        Vec<(
            management_routes::key_routes::KeyManagementRoute,
            &'static str,
        )>,
    ),
    Routes(Vec<&'static str>),
}

fn values(group: &Group) -> Output {
    match group {
        Group::KeyEnum => Output::Keys(
            management_routes::key_routes::KEY_ENUM
                .iter()
                .map(|route| (*route, route.value()))
                .collect(),
        ),
        Group::MasterKeyOnlyRoutes => {
            Output::Routes(management_routes::MASTER_KEY_ONLY_ROUTES.to_vec())
        }
        Group::KeyManagementRoutes => {
            Output::Routes(management_routes::KEY_MANAGEMENT_ROUTES.to_vec())
        }
        Group::TeamServiceAccountKeyRoutes => {
            Output::Routes(management_routes::TEAM_SERVICE_ACCOUNT_ROUTES.to_vec())
        }
        Group::ManagementRoutes => Output::Routes(management_routes::management_routes()),
        Group::SpendTrackingRoutes => {
            Output::Routes(management_routes::SPEND_TRACKING_ROUTES.to_vec())
        }
        Group::GlobalSpendTrackingRoutes => {
            Output::Routes(management_routes::GLOBAL_SPEND_TRACKING_ROUTES.to_vec())
        }
        Group::InternalUserRoutes => Output::Routes(management_routes::internal_user_routes()),
        Group::InternalUserViewOnlyRoutes => {
            Output::Routes(management_routes::INTERNAL_USER_VIEW_ONLY_ROUTES.to_vec())
        }
        Group::OrgAdminOnlyRoutes => {
            Output::Routes(management_routes::ORG_ADMIN_ONLY_ROUTES.to_vec())
        }
        Group::AdminViewerRoutes => Output::Routes(management_routes::admin_viewer_routes()),
        Group::OrgAdminAllowedRoutes => {
            Output::Routes(management_routes::org_admin_allowed_routes())
        }
    }
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
