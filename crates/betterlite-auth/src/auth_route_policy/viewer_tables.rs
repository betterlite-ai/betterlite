// RouteChecks._PROXY_ADMIN_VIEW_ONLY_BLOCKED_ROUTES: management writes a view-only admin may never call
pub(crate) const EARLY_BLOCKED_ROUTES: &[&str] = &[
    "/user/new",
    "/management/v1/users/bulk",
    "/user/delete",
    "/management/v1/users/bulk_delete",
    "/user/bulk_update",
    "/team/new",
    "/management/v1/teams/{team_id}/members/bulk_delete",
    "/management/v1/teams/{team_id}/members/bulk_update",
    "/team/update",
    "/team/delete",
    "/team/block",
    "/team/unblock",
    "/team/permissions_update",
    "/team/permissions_bulk_update",
    "/model/new",
    "/model/update",
    "/model/delete",
    "/jwt/key/mapping/new",
    "/jwt/key/mapping/update",
    "/jwt/key/mapping/delete",
    "/key/generate",
    "/key/update",
    "/key/delete",
    "/key/regenerate",
    "/key/service-account/generate",
    "/key/block",
    "/key/unblock",
    "/key/bulk_update",
    "/team/key/bulk_update",
];

// RouteChecks._PROXY_ADMIN_VIEW_ONLY_BLOCKED_KEY_SUFFIXES: the write past a /key/{key_id} template
pub(crate) const EARLY_BLOCKED_KEY_SUFFIXES: [&str; 2] = ["/regenerate", "/reset_spend"];

// RouteChecks._SAFE_HTTP_METHODS: read verbs a view-only admin keeps on every route
pub(crate) const SAFE_HTTP_METHODS: [&str; 3] = ["GET", "HEAD", "OPTIONS"];

// RouteChecks._ADMIN_VIEWER_BLOCKED_WRITE_ROUTES: the backstop when a write arrives with a safe-looking shape
pub(crate) const LATE_BLOCKED_WRITE_ROUTES: &[&str] = &[
    "/user/new",
    "/management/v1/users/bulk",
    "/user/delete",
    "/management/v1/users/bulk_delete",
    "/user/bulk_update",
    "/team/new",
    "/management/v1/teams/{team_id}/members/bulk_delete",
    "/management/v1/teams/{team_id}/members/bulk_update",
    "/team/update",
    "/team/delete",
    "/model/new",
    "/model/update",
    "/model/delete",
    "/key/generate",
    "/key/delete",
    "/key/update",
    "/key/regenerate",
    "/key/service-account/generate",
    "/key/block",
    "/key/unblock",
    "/team/key/bulk_update",
];

pub(crate) const LATE_BLOCKED_KEY_SUFFIX: &str = "/regenerate";
