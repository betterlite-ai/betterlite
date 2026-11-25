#![cfg(test)]

use std::future::{Future, ready};

use crate::gateway_dcr::ports::{
    ConsentTeam, LookupConsentTeams, MintedProxyCredential, ProxyCredentialMintFailure,
    RefuseProxyCredential, RefuseSubjectToken, ReloadUserFailure, SubjectTokenRefusal,
    UnavailableVendorCredential, UnreachableServer, VendorCredentialState,
};
use crate::gateway_dcr::subject_token::{ExchangeSubjectToken as _, RefusalCode};

fn assert_send_sync<T>()
where
    T: Send + Sync,
{
}

struct Teams;

impl LookupConsentTeams for Teams {
    fn consent_teams(
        &self,
        _user_id: &str,
    ) -> impl Future<Output = Result<Vec<ConsentTeam>, ReloadUserFailure>> + Send {
        ready(Ok(vec![ConsentTeam {
            team_id: "team-1".to_owned(),
            team_alias: Some("Team One".to_owned()),
        }]))
    }
}

#[test]
fn refusing_defaults_are_send_and_sync() {
    assert_send_sync::<RefuseProxyCredential>();
    assert_send_sync::<RefuseSubjectToken>();
    assert_send_sync::<UnavailableVendorCredential>();
    assert_send_sync::<UnreachableServer>();
}

#[test]
fn port_values_are_send_and_sync() {
    assert_send_sync::<MintedProxyCredential>();
    assert_send_sync::<ConsentTeam>();
    assert_send_sync::<SubjectTokenRefusal>();
    assert_send_sync::<ReloadUserFailure>();
    assert_send_sync::<VendorCredentialState>();
    assert_send_sync::<ProxyCredentialMintFailure>();
}

#[tokio::test]
async fn refuse_subject_token_serves_every_context_type() {
    let unit = RefuseSubjectToken
        .exchange("subject", &())
        .await
        .unwrap_err();
    let text = RefuseSubjectToken
        .exchange("subject", &"context".to_owned())
        .await
        .unwrap_err();
    assert_eq!(unit.error, RefusalCode::UnsupportedGrantType);
    assert_eq!(text.error, unit.error);
    assert_eq!(text.description, unit.description);
}

#[tokio::test]
async fn a_port_implementation_answers_with_a_ready_future() {
    let teams = Teams.consent_teams("user-1").await.unwrap();
    assert_eq!(
        teams,
        vec![ConsentTeam {
            team_id: "team-1".to_owned(),
            team_alias: Some("Team One".to_owned()),
        }]
    );
}
