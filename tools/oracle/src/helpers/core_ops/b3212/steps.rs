use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_auth::auth_identity::records::Principal;
use betterlite_core::api::stored_skill_reads::StoredSkillReads;
use betterlite_core::api::stored_skill_reads::error::StoredSkillReadError;
use betterlite_core::api::stored_skill_reads::stored_skill::StoredSkill;
use betterlite_core::skills::store::{DEFAULT_PAGE_LIMIT, DEFAULT_PAGE_OFFSET, SkillPage};
use serde_json::{Value, json};

use super::Step;
use super::principal::principal_of;
use super::scripted::Scripted;
use betterlite_oracle_helper_api::error::HelperError;

fn page_of(step: &Step) -> SkillPage {
    SkillPage {
        limit: step.limit.unwrap_or(DEFAULT_PAGE_LIMIT),
        offset: step.offset.unwrap_or(DEFAULT_PAGE_OFFSET),
    }
}

const fn token(error: &StoredSkillReadError) -> &'static str {
    match error {
        StoredSkillReadError::NotFound { .. } => "not_found",
        StoredSkillReadError::NoDatabase => "no_database",
        StoredSkillReadError::Repository { .. }
        | StoredSkillReadError::FileContent(_)
        | StoredSkillReadError::Cache(_) => "exception",
    }
}

fn skill_json(skill: &StoredSkill) -> Value {
    json!({
        "skill_id": &skill.skill_id,
        "display_title": &skill.display_title,
        "description": &skill.description,
        "instructions": &skill.instructions,
        "source": &skill.source,
        "latest_version": &skill.latest_version,
        "file_content": skill.file_content.as_deref().map(|bytes| STANDARD.encode(bytes)),
        "file_name": &skill.file_name,
        "file_type": &skill.file_type,
        "metadata": &skill.metadata,
        "created_at": &skill.created_at,
        "created_by": &skill.created_by,
        "updated_at": &skill.updated_at,
        "updated_by": &skill.updated_by,
    })
}

pub(super) async fn run_steps(
    reads: &StoredSkillReads<Scripted>,
    scripted: &Scripted,
    steps: &[Step],
) -> Result<Vec<Value>, Box<HelperError>> {
    let mut results = Vec::with_capacity(steps.len());
    for step in steps {
        if let Some(available) = step.available {
            scripted.set_available(available);
        }
        let caller = step.principal.as_ref().map(principal_of).transpose()?;
        let answer = match step.action.as_str() {
            "list" => list_answer(reads.list_skills(page_of(step), caller.as_ref()).await),
            "get" => skill_answer(reads, step, caller.as_ref()).await,
            other => json!({"action": other, "error": "invalid_input"}),
        };
        results.push(answer);
    }
    Ok(results)
}

fn list_answer(listed: Result<Vec<StoredSkill>, StoredSkillReadError>) -> Value {
    match listed {
        Ok(skills) => {
            let rows = Value::Array(skills.iter().map(skill_json).collect());
            json!({"action": "list", "rows": rows})
        }
        Err(error) => json!({"action": "list", "error": token(&error)}),
    }
}

async fn skill_answer(
    reads: &StoredSkillReads<Scripted>,
    step: &Step,
    caller: Option<&Principal>,
) -> Value {
    let action = step.action.as_str();
    let Some(skill_id) = step.skill_id.as_deref() else {
        return json!({"action": action, "error": "invalid_input"});
    };
    match reads.get_skill(skill_id, caller).await {
        Ok(skill) => json!({"action": action, "row": skill_json(&skill)}),
        Err(error) => json!({"action": action, "error": token(&error)}),
    }
}
