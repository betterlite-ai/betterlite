use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_auth::auth_identity::records::Principal;
use betterlite_auth::auth_model_patterns::decision::ModelAccessContext;
use betterlite_auth::auth_model_patterns::denial::ModelAccessDecision;
use betterlite_auth::auth_model_patterns::pattern::UnresolvedProviderInference;
use betterlite_auth::auth_model_patterns::user_model::can_user_call_model;
use betterlite_auth::auth_model_patterns::view::ModelAccessView;
use betterlite_auth::auth_request_models::RequestedModels;
use betterlite_auth::user_roles::UserRole;
use betterlite_db::auxiliary_repository::repository::PrismaClient;
use betterlite_db::budget_permission_rows::{JsonData, LiteLLMBudgetTable};
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::key_user_writes::json_text::SerdeJsonEncoder;
use betterlite_db::key_user_writes::json_text::jsonify_object;
use betterlite_db::model_secrets::DEFAULT_USER_ID;
use betterlite_db::postgres_store::session::PostgresStore;

use crate::key_patch_fields::soft_budget::{MAX_BUDGET, SOFT_BUDGET, validate_budget};
use crate::management_guards::metadata::set_object_metadata_field;
use crate::object_permissions::update::set_object_permission;
use crate::organization_create::access::read_user_object;
use crate::organization_create::budget::{
    budget_write_payload, created_budget_row, org_metadata_fields,
};
use crate::organization_create::caller::OrgCaller;
use crate::organization_create::error::OrganizationCreateError;
use crate::organization_create::records::{
    NewOrganizationRequest, NewOrganizationResponse, OrganizationRow, organization_row,
    request_amount,
};
use crate::organization_create::store::OrganizationStore;

pub struct OrganizationCreateState {
    pub database: Option<Arc<PostgresStore>>,
    pub router_configured: bool,
    pub view: Arc<dyn ModelAccessView>,
    pub max_recurse_depth: i64,
}

pub async fn create_organization(
    state: &OrganizationCreateState,
    principal: &Principal,
    request: NewOrganizationRequest,
) -> Result<NewOrganizationResponse, OrganizationCreateError> {
    let caller = OrgCaller::from_principal(principal).map_err(OrganizationCreateError::Caller)?;
    let Some(store) = state.database.as_deref() else {
        return Err(OrganizationCreateError::DatabaseNotConnected);
    };
    check_guards(state, &caller, &request)?;
    let user_object = read_user_object(store, &caller).await;
    let mut fields = request
        .fields()
        .map_err(OrganizationCreateError::Encoding)?;
    let availability = Availability::default();
    let prisma = PrismaClient {
        writer: store,
        reader: store,
        availability: &availability,
    };
    let organization_store = OrganizationStore { client: prisma };
    let actor = caller
        .user_id
        .clone()
        .unwrap_or_else(|| String::from(DEFAULT_USER_ID));
    let budget_table = attach_budget(&organization_store, &request, &mut fields, &actor).await?;
    set_object_permission(&mut fields, Some(prisma), &[]).await?;
    check_request_models(state, &caller, &request, user_object.as_ref())?;
    let metadata = partition_metadata(&request, &mut fields);
    let organization = organization_row(&request, &fields, metadata.as_ref(), &actor)?;
    let record = jsonify_object(&organization, &SerdeJsonEncoder);
    let logged =
        serde_json::to_string_pretty(&record).map_err(OrganizationCreateError::Encoding)?;
    tracing::info!("new_organization_row: {logged}");
    let created = organization_store.create_organization(record).await?;
    let row = organization_row_of(&created)?;
    Ok(response_of(row, &request, metadata, budget_table))
}

// the role, router and money guards in LiteLLM's order, before any read or write
fn check_guards(
    state: &OrganizationCreateState,
    caller: &OrgCaller,
    request: &NewOrganizationRequest,
) -> Result<(), OrganizationCreateError> {
    if caller.user_role != Some(UserRole::ProxyAdmin) {
        return Err(OrganizationCreateError::NotAdmin {
            role: caller.role_text(),
        });
    }
    if !state.router_configured {
        return Err(OrganizationCreateError::NoRouter);
    }
    validate_budget(
        MAX_BUDGET,
        request_amount(MAX_BUDGET, request.max_budget.as_ref())?,
    )?;
    validate_budget(
        SOFT_BUDGET,
        request_amount(SOFT_BUDGET, request.soft_budget.as_ref())?,
    )?;
    let _increase = request_amount(
        "temp_budget_increase",
        request.temp_budget_increase.as_ref(),
    )?;
    Ok(())
}

// the budget the organization points at: one built from the request, or the stored row an id names
async fn attach_budget(
    organization_store: &OrganizationStore<'_>,
    request: &NewOrganizationRequest,
    fields: &mut BTreeMap<String, JsonData>,
    actor: &str,
) -> Result<Option<LiteLLMBudgetTable>, OrganizationCreateError> {
    let Some(budget_id) = request.budget_id.as_deref() else {
        let budget_data = budget_write_payload(fields, actor);
        let created = organization_store.create_budget(budget_data).await?;
        let created_id = string_column(&created, "budget_id")?;
        fields.insert("budget_id".to_owned(), JsonData::String(created_id.clone()));
        return Ok(Some(created_budget_row(&created_id, request)));
    };
    Ok(organization_store.find_budget(budget_id).await?)
}

fn string_column(row: &JsonData, name: &'static str) -> Result<String, OrganizationCreateError> {
    let JsonData::Object(fields) = row else {
        return Err(OrganizationCreateError::NotAnObject);
    };
    match fields.get(name) {
        Some(JsonData::String(value)) => Ok(value.clone()),
        _ => Err(OrganizationCreateError::MissingColumn { name }),
    }
}

// a caller with no model list keeps every model; a restricted caller may not hand over the whole router
fn check_request_models(
    state: &OrganizationCreateState,
    caller: &OrgCaller,
    request: &NewOrganizationRequest,
    user_object: Option<&LiteLLMUserTable>,
) -> Result<(), OrganizationCreateError> {
    if caller.models.is_empty() {
        return Ok(());
    }
    if request.models.is_empty() {
        return Err(OrganizationCreateError::AllModelsDenied);
    }
    check_models(state, &request.models, user_object)
}

// the payload's metadata column: the request's metadata plus every _ORG_METADATA_FIELDS value it sent
fn partition_metadata(
    request: &NewOrganizationRequest,
    fields: &mut BTreeMap<String, JsonData>,
) -> Option<BTreeMap<String, JsonData>> {
    let mut metadata = request.metadata.clone();
    for name in org_metadata_fields() {
        if let Some(value) = fields.remove(name) {
            set_object_metadata_field(&mut metadata, name, value);
        }
    }
    metadata
}

fn check_models(
    state: &OrganizationCreateState,
    models: &[String],
    user_object: Option<&LiteLLMUserTable>,
) -> Result<(), OrganizationCreateError> {
    let provider = UnresolvedProviderInference;
    let aliases = BTreeMap::new();
    let context = ModelAccessContext {
        view: Some(state.view.as_ref()),
        global_model_aliases: &aliases,
        compaction: None,
        provider: &provider,
        max_recurse_depth: state.max_recurse_depth,
    };
    let key_aliases = BTreeMap::new();
    for requested in models {
        match can_user_call_model(
            &RequestedModels::One(requested.clone()),
            user_object,
            &key_aliases,
            &context,
        ) {
            ModelAccessDecision::Allowed => {}
            ModelAccessDecision::Denied(denial) => {
                return Err(OrganizationCreateError::ModelDenied(denial));
            }
            ModelAccessDecision::RecurseDepthExceeded { model } => {
                return Err(OrganizationCreateError::RecurseDepth { model });
            }
        }
    }
    Ok(())
}

pub fn organization_row_of(created: &JsonData) -> Result<OrganizationRow, OrganizationCreateError> {
    serde_json::to_value(created)
        .and_then(serde_json::from_value::<OrganizationRow>)
        .map_err(OrganizationCreateError::Row)
}

// NewOrganizationResponse from the stored row, with the budget table the include would return
#[must_use]
pub fn response_of(
    row: OrganizationRow,
    request: &NewOrganizationRequest,
    metadata: Option<BTreeMap<String, JsonData>>,
    budget_table: Option<LiteLLMBudgetTable>,
) -> NewOrganizationResponse {
    NewOrganizationResponse {
        organization_id: row.organization_id,
        organization_alias: row.organization_alias,
        budget_id: row.budget_id,
        spend: row.spend,
        metadata: Some(metadata.unwrap_or_default()),
        models: request.models.clone(),
        model_spend: BTreeMap::new(),
        created_by: row.created_by,
        updated_by: row.updated_by,
        users: None,
        litellm_budget_table: budget_table,
        object_permission: None,
        object_permission_id: row.object_permission_id,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}
