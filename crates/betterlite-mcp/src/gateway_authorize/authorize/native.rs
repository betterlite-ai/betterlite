use betterlite_db::stored_cipher::DbCipher;

use super::{
    AuthorizeRequest, NewConnectFlow, login_redirect, new_connect_flow, rejected_authorize_request,
    set_flow_cookie,
};
use crate::gateway_authorize::oauth_error;
use crate::gateway_authorize::reply::{
    FlowTokenSource, GatewayReply, RenderConsentPage, RequestView,
};
use crate::gateway_authorize::url_parts::origin_only;
use crate::gateway_authorize::{is_loopback_uri, no_cache_headers};
use crate::gateway_dcr::contracts::{
    DB_FAULTED_DESCRIPTION, DB_UNAVAILABLE_DESCRIPTION, PROXY_API_AUDIENCE,
};
use crate::gateway_dcr::oauth_error::OauthError;
use crate::gateway_dcr::oauth_error::OauthErrorCode::{
    AccessDenied, InvalidRequest, ServerError, TemporarilyUnavailable,
};
use crate::gateway_dcr::ports::{ConsentTeam, LookupConsentTeams, ReloadUserFailure};

const LOOPBACK_DESCRIPTION: &str = "a proxy-API grant may only redirect to a loopback address";

pub struct NativeClientAuthorize<'request> {
    pub view: &'request RequestView,
    pub request: &'request AuthorizeRequest<'request>,
    pub session_user_id: Option<&'request str>,
    pub now: i64,
}

pub async fn native_client_authorize<T>(
    call: &NativeClientAuthorize<'_>,
    teams: &T,
    page: &dyn RenderConsentPage,
    cipher: &DbCipher,
    tokens: &dyn FlowTokenSource,
) -> GatewayReply
where
    T: LookupConsentTeams + ?Sized,
{
    match native_authorize(call, teams, page, cipher, tokens).await {
        Ok(reply) => reply,
        Err(error) => GatewayReply::Error(error),
    }
}

async fn native_authorize<T>(
    call: &NativeClientAuthorize<'_>,
    teams: &T,
    page: &dyn RenderConsentPage,
    cipher: &DbCipher,
    tokens: &dyn FlowTokenSource,
) -> Result<GatewayReply, OauthError>
where
    T: LookupConsentTeams + ?Sized,
{
    if let Some(error) = rejected_authorize_request(call.request, cipher) {
        return Err(error);
    }
    if !is_loopback_uri(call.request.redirect_uri) {
        return Err(oauth_error(400, InvalidRequest, LOOPBACK_DESCRIPTION));
    }
    let Some(user_id) = call.session_user_id else {
        return Ok(login_redirect(
            &call.view.base_url,
            &call.view.path,
            call.view.query.as_deref(),
        ));
    };
    let consent = match teams.consent_teams(user_id).await {
        Ok(consent) => consent,
        Err(failure) => return Err(consent_lookup_failure(failure)),
    };
    let handle = tokens.fresh_token();
    let flow = new_connect_flow(
        NewConnectFlow {
            user_id,
            client_id: call.request.client_id,
            redirect_uri: call.request.redirect_uri,
            state: call.request.state,
            code_challenge: call.request.code_challenge.unwrap_or_default().to_owned(),
            resource_server_id: None,
            audience: Some(PROXY_API_AUDIENCE),
        },
        tokens,
        call.now,
    );
    let client_origin = origin_only(call.request.redirect_uri)?;
    let complete_url = format!("{}/authorize/complete", call.view.base_url);
    let body = page.render(
        &client_origin,
        user_id,
        &team_labels(&consent),
        &handle,
        &complete_url,
    );
    let cookie = set_flow_cookie(&handle, &flow, &call.view.base_url, cipher)?;
    Ok(GatewayReply::Html {
        body,
        headers: consent_headers(),
        set_cookie: Some(Box::new(cookie)),
        delete_cookie: None,
    })
}

fn consent_lookup_failure(failure: ReloadUserFailure) -> OauthError {
    match failure {
        ReloadUserFailure::Unavailable => {
            oauth_error(503, TemporarilyUnavailable, DB_UNAVAILABLE_DESCRIPTION)
        }
        ReloadUserFailure::Faulted => {
            oauth_error(503, TemporarilyUnavailable, DB_FAULTED_DESCRIPTION)
        }
        ReloadUserFailure::Unresolvable => oauth_error(
            500,
            ServerError,
            "the gateway is not configured to resolve users",
        ),
        ReloadUserFailure::NoActiveKey => {
            oauth_error(403, AccessDenied, "the signed-in user is not active")
        }
    }
}

fn team_labels(teams: &[ConsentTeam]) -> Vec<(String, String)> {
    teams
        .iter()
        .map(|team| {
            let alias = team.team_alias.as_deref().filter(|alias| !alias.is_empty());
            let label = alias.unwrap_or(team.team_id.as_str());
            (team.team_id.clone(), label.to_owned())
        })
        .collect()
}

fn consent_headers() -> Vec<(String, String)> {
    let mut headers = no_cache_headers();
    headers.push(("X-Frame-Options".to_owned(), "DENY".to_owned()));
    headers.push((
        "Content-Security-Policy".to_owned(),
        "frame-ancestors 'none'".to_owned(),
    ));
    headers
}
