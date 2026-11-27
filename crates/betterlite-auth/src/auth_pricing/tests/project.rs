use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::PoisonError;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use async_trait::async_trait;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::resource_rows::project::ProjectTable;
use betterlite_error::proxy_error_types::ModelAccessObject;
use rust_decimal::Decimal;
use serde::Serialize;

use crate::auth_key_budgets::spend::{SpendReadError, SpendReader};
use crate::auth_model_patterns::decision::ModelAccessContext;
use crate::auth_model_patterns::denial::model_access_denied;
use crate::auth_model_patterns::pattern::UnresolvedProviderInference;
use crate::auth_prefetch::rows::CachedProject;
use crate::auth_pricing::project::{
    DeploymentProjectGrant, ProjectCollaborators, ProjectError, ProjectModelGrant,
    run_project_checks,
};
use crate::auth_request_models::RequestedModels;
use crate::auth_tenant_budgets::TenantAdmission;
use crate::auth_tenant_budgets::alerts::{TenantAlert, TenantAlertKind, TenantAlertSink};
use crate::current_spend::SpendQuery;
use crate::entity_model_access::error::EntityModelAccessError;
use crate::resolved_key::auth::{AuthInput, UserAPIKeyAuth};

#[derive(Debug)]
struct ScriptedSpend {
    spend: Decimal,
    reads: AtomicUsize,
}

impl ScriptedSpend {
    fn new(spend: &str) -> Self {
        Self {
            spend: spend.parse().expect("the test spend parses"),
            reads: AtomicUsize::new(0),
        }
    }

    fn reads(&self) -> usize {
        self.reads.load(Ordering::Acquire)
    }
}

#[async_trait]
impl SpendReader for ScriptedSpend {
    async fn read_spend(&self, _query: &SpendQuery<'_>) -> Result<Decimal, SpendReadError> {
        self.reads.fetch_add(1, Ordering::AcqRel);
        Ok(self.spend)
    }
}

#[derive(Debug, Default)]
struct AlertRecorder {
    kinds: Mutex<Vec<TenantAlertKind>>,
}

impl AlertRecorder {
    fn kinds(&self) -> Vec<TenantAlertKind> {
        self.kinds
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl TenantAlertSink for AlertRecorder {
    fn schedule(&self, alert: TenantAlert) {
        self.kinds
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(alert.kind);
    }
}

#[derive(Debug, Default)]
struct GrantRecorder {
    calls: AtomicUsize,
    deny: AtomicBool,
}

impl GrantRecorder {
    fn denying() -> Self {
        Self {
            calls: AtomicUsize::new(0),
            deny: AtomicBool::new(true),
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::Acquire)
    }
}

impl ProjectModelGrant for GrantRecorder {
    fn check(
        &self,
        _model: &RequestedModels,
        _project_row: &ProjectTable,
        _alias_targets: &BTreeMap<String, String>,
    ) -> Result<(), EntityModelAccessError> {
        self.calls.fetch_add(1, Ordering::AcqRel);
        if self.deny.load(Ordering::Acquire) {
            return Err(EntityModelAccessError::Denied(Box::new(
                model_access_denied(
                    ModelAccessObject::Project,
                    &["allowed".to_owned()],
                    "requested",
                ),
            )));
        }
        Ok(())
    }
}

#[derive(Serialize)]
struct TokenFields<'fields> {
    token: &'fields str,
    #[serde(skip_serializing_if = "Option::is_none")]
    org_id: Option<&'fields str>,
}

fn token(text: &str) -> UserAPIKeyAuth {
    let fields = TokenFields {
        token: text,
        org_id: Some("org-9"),
    };
    let encoded = serde_json::to_value(fields).expect("the token fields encode");
    let decoded: JsonData = serde_json::from_value(encoded).expect("the token fields decode");
    let JsonData::Object(map) = decoded else {
        panic!("the token fields must be an object");
    };
    UserAPIKeyAuth::construct(AuthInput::Dict(map)).expect("the token constructs")
}

fn cached_project(
    project_id: &str,
    blocked: bool,
    models: &[&str],
    budget: Option<(&str, &str)>,
) -> CachedProject {
    let models = serde_json::to_string(models).expect("the model list encodes");
    let budget_table = budget.map_or_else(
        || "null".to_owned(),
        |(max_budget, soft_budget)| {
            format!(r#"{{"budget_id":"b1","max_budget":{max_budget},"soft_budget":{soft_budget}}}"#)
        },
    );
    let row = format!(
        r#"{{"project_id":"{project_id}","blocked":{blocked},"models":{models},"spend":6,"litellm_budget_table":{budget_table}}}"#
    );
    serde_json::from_str(&row).expect("the project row decodes")
}

fn admission<'deps>(
    reader: &'deps ScriptedSpend,
    alerts: &'deps AlertRecorder,
) -> TenantAdmission<'deps> {
    TenantAdmission {
        reader,
        alerts: Some(alerts),
    }
}

fn one(model: &str) -> RequestedModels {
    RequestedModels::One(model.to_owned())
}

#[tokio::test]
async fn blocked_project_first() {
    let reader = ScriptedSpend::new("6");
    let alerts = AlertRecorder::default();
    let grants = GrantRecorder::denying();
    let project = cached_project("proj-1", true, &["allowed"], Some(("5", "0")));
    let error = run_project_checks(
        Some(&project),
        &one("requested"),
        false,
        None,
        &ProjectCollaborators {
            grant: &grants,
            admission: admission(&reader, &alerts),
        },
    )
    .await
    .expect_err("a blocked project stops the request");
    assert_eq!(
        error.to_string(),
        "Project=proj-1 is blocked. Update via `/project/update` if you're an admin."
    );
    assert!(matches!(error, ProjectError::Blocked { project_id } if project_id == "proj-1"));
    assert_eq!(grants.calls(), 0);
    assert_eq!(reader.reads(), 0);
    assert!(alerts.kinds().is_empty());
}

#[tokio::test]
async fn grant_before_budgets() {
    let reader = ScriptedSpend::new("6");
    let alerts = AlertRecorder::default();
    let grants = GrantRecorder::denying();
    let auth = token("sk-grant");
    let project = cached_project("proj-1", false, &["allowed"], Some(("0", "0")));
    let error = run_project_checks(
        Some(&project),
        &one("requested"),
        false,
        Some(&auth),
        &ProjectCollaborators {
            grant: &grants,
            admission: admission(&reader, &alerts),
        },
    )
    .await
    .expect_err("the model grant denies before any budget is read");
    assert!(matches!(error, ProjectError::Grant(_)));
    assert_eq!(grants.calls(), 1);
    assert_eq!(reader.reads(), 0);
    assert!(alerts.kinds().is_empty());
}

#[tokio::test]
async fn hard_before_soft() {
    let reader = ScriptedSpend::new("6");
    let alerts = AlertRecorder::default();
    let grants = GrantRecorder::default();
    let auth = token("sk-hard");
    let project = cached_project("proj-1", false, &["allowed"], Some(("5", "0")));
    let error = run_project_checks(
        Some(&project),
        &one("allowed"),
        false,
        Some(&auth),
        &ProjectCollaborators {
            grant: &grants,
            admission: admission(&reader, &alerts),
        },
    )
    .await
    .expect_err("the hard budget denies before the soft check");
    assert!(matches!(error, ProjectError::Budget(_)));
    assert_eq!(grants.calls(), 1);
    assert_eq!(reader.reads(), 1);
    assert_eq!(alerts.kinds(), vec![TenantAlertKind::ProjectBudget]);
}

#[tokio::test]
async fn skip_keeps_model_check() {
    let reader = ScriptedSpend::new("6");
    let alerts = AlertRecorder::default();
    let denying = GrantRecorder::denying();
    let auth = token("sk-skip");
    let project = cached_project("proj-1", false, &["allowed"], Some(("0", "0")));
    let error = run_project_checks(
        Some(&project),
        &one("requested"),
        true,
        Some(&auth),
        &ProjectCollaborators {
            grant: &denying,
            admission: admission(&reader, &alerts),
        },
    )
    .await
    .expect_err("the model grant still runs when budgets are skipped");
    assert!(matches!(error, ProjectError::Grant(_)));
    assert_eq!(denying.calls(), 1);
    assert_eq!(reader.reads(), 0);

    let allowing = GrantRecorder::default();
    run_project_checks(
        Some(&project),
        &one("allowed"),
        true,
        Some(&auth),
        &ProjectCollaborators {
            grant: &allowing,
            admission: admission(&reader, &alerts),
        },
    )
    .await
    .expect("a skipped budget cannot deny");
    assert_eq!(allowing.calls(), 1);
    assert_eq!(reader.reads(), 0);
    assert!(alerts.kinds().is_empty());

    // the model gate runs only for a truthy request model and a nonempty project list
    let quiet = GrantRecorder::denying();
    for skipped in [
        RequestedModels::None,
        RequestedModels::Many(Vec::new()),
        one(""),
    ] {
        run_project_checks(
            Some(&project),
            &skipped,
            true,
            Some(&auth),
            &ProjectCollaborators {
                grant: &quiet,
                admission: admission(&reader, &alerts),
            },
        )
        .await
        .expect("an absent or empty request model skips the grant");
    }
    let unrestricted = cached_project("proj-2", false, &[], Some(("0", "0")));
    run_project_checks(
        Some(&unrestricted),
        &one("allowed"),
        true,
        Some(&auth),
        &ProjectCollaborators {
            grant: &quiet,
            admission: admission(&reader, &alerts),
        },
    )
    .await
    .expect("a project without a model list skips the grant");
    assert_eq!(quiet.calls(), 0);
    assert_eq!(reader.reads(), 0);

    run_project_checks(
        None,
        &one("allowed"),
        false,
        Some(&auth),
        &ProjectCollaborators {
            grant: &allowing,
            admission: admission(&reader, &alerts),
        },
    )
    .await
    .expect("a missing project skips every check");
    assert_eq!(allowing.calls(), 1);
    assert_eq!(reader.reads(), 0);
}

#[test]
fn deployment_grant_checks_project_models() {
    let aliases = BTreeMap::new();
    let provider = UnresolvedProviderInference;
    let context = ModelAccessContext {
        view: None,
        global_model_aliases: &aliases,
        compaction: None,
        provider: &provider,
        max_recurse_depth: 100,
    };
    let grant = DeploymentProjectGrant { context };
    assert_eq!(format!("{grant:?}"), "DeploymentProjectGrant");
    let no_aliases = BTreeMap::new();
    let project = cached_project("proj-1", false, &["allowed"], None);
    grant
        .check(&one("allowed"), &project.project, &no_aliases)
        .expect("a model the project lists passes");
    let error = grant
        .check(&one("other"), &project.project, &no_aliases)
        .expect_err("a model outside the project list is denied");
    assert!(matches!(error, EntityModelAccessError::Denied(_)));
    let mut alias_targets = BTreeMap::new();
    alias_targets.insert("requested".to_owned(), "allowed".to_owned());
    grant
        .check(&one("requested"), &project.project, &alias_targets)
        .expect("the key alias target is checked against the project list");
}
