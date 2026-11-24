use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use axum::body::{Body, to_bytes};
use axum::extract::Request;
use axum::http::{Method, StatusCode};
use betterlite_auth::auth_identity::records::{
    CredentialRef, NetworkContext, Principal, SourceKey,
};
use betterlite_auth::auth_identity::roles::{AuthMethod, PrincipalType};
use betterlite_db::postgres_store::errors::DbError;
use rust_decimal::Decimal;
use tower::ServiceExt as _;

use crate::auth::auth_identity::AuthContext;
use crate::spend::global_report::rows::GlobalReport;
use crate::spend::global_report::store::{
    GlobalReportHandle, GlobalReportStore, ReportBranch, ReportWindow,
};
use crate::spend::reports::store::ScopedReportError;

pub(super) struct FakeStore {
    pub(super) reply: GlobalReport,
    pub(super) failure: bool,
    pub(super) calls: Mutex<Vec<String>>,
}

pub(super) fn fake_store(reply: GlobalReport) -> Arc<FakeStore> {
    Arc::new(FakeStore {
        reply,
        failure: false,
        calls: Mutex::new(Vec::new()),
    })
}

impl FakeStore {
    pub(super) fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

fn describe(branch: ReportBranch<'_>) -> String {
    match branch {
        ReportBranch::Scoped { column, value } => format!("scoped({column:?},{value})"),
        ReportBranch::TeamCustomer {
            team_id,
            customer_id,
        } => format!("team_customer({team_id},{customer_id})"),
        ReportBranch::Teams(team_id) => format!("teams({team_id:?})"),
        ReportBranch::Customers => String::from("customers"),
        ReportBranch::Keys => String::from("keys"),
    }
}

#[async_trait]
impl GlobalReportStore for FakeStore {
    async fn report(
        &self,
        branch: ReportBranch<'_>,
        window: ReportWindow<'_>,
    ) -> Result<GlobalReport, ScopedReportError> {
        let call = format!(
            "{}@{},{}",
            describe(branch),
            window.start.to_rfc3339(),
            window.end.to_rfc3339()
        );
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
        if self.failure {
            return Err(ScopedReportError::Database(DbError::NoDbConnected));
        }
        Ok(self.reply.clone())
    }
}

fn caller() -> Principal {
    Principal {
        principal_type: PrincipalType::Human,
        subject: String::from("master_key"),
        issuer: None,
        audience: Vec::new(),
        user: None,
        organization: None,
        teams: Vec::new(),
        project: None,
        end_user: None,
        roles: Vec::new(),
        scopes: Vec::new(),
        auth_method: AuthMethod::ApiKey,
        credential_ref: CredentialRef::default(),
        network: NetworkContext::default(),
        source_key: SourceKey(None),
    }
}

pub(super) struct Reply {
    pub(super) status: StatusCode,
    pub(super) body: String,
}

pub(super) async fn call(
    query: &str,
    authenticated: bool,
    store: Option<&Arc<FakeStore>>,
) -> Reply {
    let handle: GlobalReportHandle = store.map(|fake| {
        let shared: Arc<dyn GlobalReportStore> = Arc::<FakeStore>::clone(fake);
        shared
    });
    let mut request = Request::builder()
        .method(Method::GET)
        .uri(format!("/global/spend/report?{query}"))
        .body(Body::empty())
        .unwrap();
    if authenticated {
        request.extensions_mut().insert(AuthContext {
            principal: caller(),
        });
    }
    let response = super::super::routes(handle).oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    Reply {
        status,
        body: String::from_utf8(bytes.to_vec()).unwrap(),
    }
}

pub(super) fn decimal(text: &str) -> Decimal {
    text.parse().unwrap()
}
