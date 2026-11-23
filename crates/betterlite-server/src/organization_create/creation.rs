use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_auth::auth_identity::records::Principal;
use betterlite_auth::auth_model_patterns::decision::ModelAccessContext;
use betterlite_auth::auth_model_patterns::denial::ModelAccessDecision;
use betterlite_auth::auth_model_patterns::pattern::UnresolvedProviderInference;
use betterlite_auth::auth_model_patterns::user_model::can_user_call_model;
use betterlite_auth::auth_model_patterns::view::ModelAccessView;
use betterlite_auth::auth_request_models::RequestedModels;
use betterlite_db::auxiliary_repository::repository::PrismaClient;
use betterlite_db::budget_permission_rows::{JsonData, LiteLLMBudgetTable};
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::key_user_writes::json_text::SerdeJsonEncoder;
use betterlite_db::key_user_writes::json_text::jsonify_object;
use betterlite_db::postgres_store::session::PostgresStore;

use crate::key_patch_fields::soft_budget::{MAX_BUDGET, SOFT_BUDGET, validate_budget};
use crate::management_guards::metadata::set_object_metadata_field;
use crate::object_permissions::update::set_object_permission;
use crate::organization_create::access::DatabaseUserLookup;
use crate::organization_create::access::OrgUserLookup as _;
use crate::organization_create::budget::org_metadata_fields;
use crate::organization_create::caller::{DEFAULT_ADMIN_USER_ID, OrgCaller};
use crate::organization_create::error::OrganizationCreateError;
use crate::organization_create::records::{
    BUDGET_SCHEMA_FIELDS, NewOrganizationRequest, NewOrganizationResponse, OrganizationRow,
    from_json_data, to_json_data,
};
use crate::organization_create::store::OrganizationStore;

pub struct OrganizationCreateState {
    pub database: Option<Arc<PostgresStore>>,
    pub router_configured: bool,
    pub view: Arc<dyn ModelAccessView>,
    pub max_recurse_depth: i64,
}

enum BudgetTable {
    Created(LiteLLMBudgetTable),
    Existing(Option<LiteLLMBudgetTable>),
}

impl BudgetTable {
    fn row(self) -> Option<LiteLLMBudgetTable> {
        match self {
            Self::Created(row) => Some(row),
            Self::Existing(row) => row,
        }
    }
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
    if !caller.is_proxy_admin() {
        return Err(OrganizationCreateError::NotAdmin {
            role: caller.role_text(),
        });
    }
    if !state.router_configured {
        return Err(OrganizationCreateError::NoRouter);
    }
    validate_budget(MAX_BUDGET, request.max_budget.map(|money| money.0))?;
    validate_budget(SOFT_BUDGET, request.soft_budget.map(|money| money.0))?;

    let user_object = match caller.user_id.as_deref() {
        Some(user_id) => match (DatabaseUserLookup { store }).find_user(user_id).await {
            Ok(found) => found,
            Err(error) => {
                // the source swallows every lookup and validation failure and creates the org without a user object
                tracing::debug!(error = %error, "the caller user row could not be read");
                None
            }
        },
        None => None,
    };

    let mut fields = request
        .fields()
        .map_err(OrganizationCreateError::Encoding)?;
    let availability = Availability::default();
    let prisma = PrismaClient {
        writer: store,
        reader: store,
        availability: &availability,
    };
    let organization_store = OrganizationStore::new(prisma);
    let actor = caller
        .user_id
        .clone()
        .unwrap_or_else(|| String::from(DEFAULT_ADMIN_USER_ID));

    let budget_table = if request.budget_id.is_none() {
        let budget_data = budget_write_payload(&fields, &actor);
        let created = organization_store.create_budget(budget_data).await?;
        let budget_id = string_column(&created, "budget_id")?;
        fields.insert("budget_id".to_owned(), JsonData::String(budget_id.clone()));
        BudgetTable::Created(created_budget_row(&budget_id, &request))
    } else {
        let budget_id = request.budget_id.clone().unwrap_or_default();
        BudgetTable::Existing(organization_store.find_budget(&budget_id).await?)
    };

    set_object_permission(&mut fields, Some(prisma), &[]).await?;

    if !caller.has_all_models() {
        if request.models.is_empty() {
            return Err(OrganizationCreateError::AllModelsDenied);
        }
        check_models(state, &request.models, user_object.as_ref())?;
    }

    let mut metadata = request.metadata.clone();
    for name in org_metadata_fields() {
        if let Some(value) = fields.remove(name) {
            set_object_metadata_field(&mut metadata, name, value);
        }
    }
    let budget_id = match fields.get("budget_id") {
        Some(JsonData::String(budget_id)) => budget_id.clone(),
        _ => String::new(),
    };
    let mut organization = BTreeMap::new();
    if let Some(JsonData::String(organization_id)) = fields.get("organization_id") {
        organization.insert(
            "organization_id".to_owned(),
            JsonData::String(organization_id.clone()),
        );
    }
    organization.insert(
        "organization_alias".to_owned(),
        JsonData::String(request.organization_alias.clone()),
    );
    organization.insert("budget_id".to_owned(), JsonData::String(budget_id));
    if let Some(metadata_fields) = &metadata {
        organization.insert(
            "metadata".to_owned(),
            JsonData::Object(metadata_fields.clone()),
        );
    }
    organization.insert(
        "models".to_owned(),
        to_json_data(&request.models).map_err(OrganizationCreateError::Encoding)?,
    );
    organization.insert("model_spend".to_owned(), JsonData::Object(BTreeMap::new()));
    organization.insert(
        "spend".to_owned(),
        to_json_data(&0.0_f64).map_err(OrganizationCreateError::Encoding)?,
    );
    organization.insert("created_by".to_owned(), JsonData::String(actor.clone()));
    organization.insert("updated_by".to_owned(), JsonData::String(actor));
    if let Some(JsonData::String(permission_id)) = fields.get("object_permission_id") {
        organization.insert(
            "object_permission_id".to_owned(),
            JsonData::String(permission_id.clone()),
        );
    }

    let record = jsonify_object(&organization, &SerdeJsonEncoder);
    let logged = serde_json::to_string_pretty(&record)
        .map_err(|error| OrganizationCreateError::Database(error.to_string()))?;
    tracing::info!("new_organization_row: {logged}");

    let created = organization_store.create_organization(record).await?;
    let row: OrganizationRow = from_json_data(&created)
        .map_err(|error| OrganizationCreateError::Row(error.to_string()))?;
    let object_permission_id = row.object_permission_id.clone();
    Ok(NewOrganizationResponse {
        organization_id: row.organization_id,
        organization_alias: row.organization_alias,
        budget_id: row.budget_id,
        spend: row.spend,
        metadata,
        models: request.models,
        model_spend: BTreeMap::new(),
        created_by: row.created_by,
        updated_by: row.updated_by,
        users: None,
        litellm_budget_table: budget_table.row(),
        object_permission: None,
        object_permission_id,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

// the request's LiteLLM_BudgetTable columns, jsonified, with the actor columns the create always writes
fn budget_write_payload(
    fields: &BTreeMap<String, JsonData>,
    actor: &str,
) -> BTreeMap<String, JsonData> {
    let sent: BTreeMap<String, JsonData> = fields
        .iter()
        .filter(|(name, _)| BUDGET_SCHEMA_FIELDS.contains(&name.as_str()))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    let mut data = jsonify_object(&sent, &SerdeJsonEncoder);
    data.insert("created_by".to_owned(), JsonData::String(actor.to_owned()));
    data.insert("updated_by".to_owned(), JsonData::String(actor.to_owned()));
    data
}

// the created row, projected onto LiteLLM_BudgetTable: allowed_models comes from the column default
fn created_budget_row(budget_id: &str, request: &NewOrganizationRequest) -> LiteLLMBudgetTable {
    LiteLLMBudgetTable {
        budget_id: Some(budget_id.to_owned()),
        soft_budget: request.soft_budget.and_then(|money| money.decimal()),
        max_budget: request.max_budget.and_then(|money| money.decimal()),
        max_parallel_requests: request.max_parallel_requests,
        tpm_limit: request.tpm_limit,
        rpm_limit: request.rpm_limit,
        tpd_limit: request.tpd_limit,
        model_max_budget: request.model_max_budget.clone(),
        budget_duration: request.budget_duration.clone(),
        allowed_models: Some(request.allowed_models.clone().unwrap_or_default()),
        temp_budget_increase: request
            .temp_budget_increase
            .and_then(|money| money.decimal()),
        temp_budget_expiry: request.temp_budget_expiry,
    }
}

fn string_column(row: &JsonData, name: &str) -> Result<String, OrganizationCreateError> {
    let JsonData::Object(fields) = row else {
        return Err(OrganizationCreateError::Row(String::from(
            "the created row is not an object",
        )));
    };
    match fields.get(name) {
        Some(JsonData::String(value)) => Ok(value.clone()),
        _ => Err(OrganizationCreateError::Row(format!(
            "the created row has no {name}"
        ))),
    }
}

fn check_models(
    state: &OrganizationCreateState,
    models: &[String],
    user_object: Option<&betterlite_db::identity_rows::user::LiteLLMUserTable>,
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
    for model in models {
        match can_user_call_model(
            &RequestedModels::One(model.clone()),
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
