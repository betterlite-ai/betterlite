use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::skills::operations::{
    HostedSkillCreation, HostedSkillOpsError, SkillUpload, SkillUploadType,
};
use crate::skills::store::repository::SkillWrite;
use crate::skills::store::{SkillPage, StoreError};

use super::support::{self, Effect, ScriptedRepository, admin, logged, number, object, principal};

fn uploads<'request>() -> [SkillUpload<'request>; 2] {
    [
        support::upload("first.zip", b"first", SkillUploadType::Defaulted),
        support::upload(
            "second.zip",
            b"second",
            SkillUploadType::Given(Some("text/plain")),
        ),
    ]
}

fn created_write(repository: &ScriptedRepository) -> SkillWrite {
    let effects = repository.effects();
    let Effect::Create(write) = &effects[0] else {
        panic!("create must reach the repository once");
    };
    (**write).clone()
}

#[test]
fn create_forwards_identity_metadata_and_caller_arguments() {
    support::runtime().block_on(async {
        let repository = ScriptedRepository::default();
        let ops = support::ops(repository.clone());
        let mut logging = support::logging_state();
        let mut metadata = IndexMap::new();
        metadata.insert("tier".to_owned(), number(2));
        let skill = ops
            .create_skill(
                &mut logging,
                HostedSkillCreation {
                    display_title: Some("Title"),
                    description: Some("Desc"),
                    instructions: Some("Do"),
                    metadata: Some(&metadata),
                    user_id: Some("bob"),
                    principal: Some(&principal("alice")),
                    litellm_call_id: Some("call-9"),
                    ..HostedSkillCreation::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(skill.id, support::SKILL_ID);
        assert_eq!(skill.created_at, support::NOW);
        assert_eq!(skill.updated_at, support::NOW);
        assert_eq!(skill.display_title.as_deref(), Some("Title"));
        assert_eq!(skill.description.as_deref(), Some("Desc"));
        assert_eq!(skill.source, "custom");
        assert_eq!(skill.r#type, "skill");
        let write = created_write(&repository);
        assert_eq!(write.display_title.as_deref(), Some("Title"));
        assert_eq!(write.description.as_deref(), Some("Desc"));
        assert_eq!(write.instructions.as_deref(), Some("Do"));
        assert_eq!(write.metadata.as_deref(), Some(r#"{"tier":2}"#));
        assert_eq!(write.created_by, "alice");
        assert_eq!(write.updated_by, "alice");
        assert_eq!(
            logged(&logging, "litellm_params"),
            object(&[("litellm_call_id", support::text("call-9"))])
        );
        assert_eq!(
            logged(&logging, "custom_llm_provider"),
            support::text("litellm_proxy")
        );
    });
}

#[test]
fn create_encodes_nested_metadata_as_row_json() {
    support::runtime().block_on(async {
        let repository = ScriptedRepository::default();
        let ops = support::ops(repository.clone());
        let mut logging = support::logging_state();
        let mut nested = IndexMap::new();
        nested.insert(
            "b".to_owned(),
            Json::List(vec![Json::Bool(true), Json::Null]),
        );
        nested.insert("a".to_owned(), number(1));
        let mut metadata = IndexMap::new();
        metadata.insert("note".to_owned(), Json::Object(nested));
        ops.create_skill(
            &mut logging,
            HostedSkillCreation {
                metadata: Some(&metadata),
                principal: Some(&principal("alice")),
                ..HostedSkillCreation::default()
            },
        )
        .await
        .unwrap();
        let write = created_write(&repository);
        assert_eq!(
            write.metadata.as_deref(),
            Some(r#"{"note":{"a":1,"b":[true,null]}}"#)
        );
    });
}

#[test]
fn create_uses_the_first_uploaded_tuple_and_ignores_later_ones() {
    support::runtime().block_on(async {
        let repository = ScriptedRepository::default();
        let ops = support::ops(repository.clone());
        let mut logging = support::logging_state();
        let files = uploads();
        ops.create_skill(
            &mut logging,
            HostedSkillCreation {
                files: Some(&files),
                file_name: Some("explicit.zip"),
                file_type: Some("application/x-explicit"),
                principal: Some(&principal("alice")),
                ..HostedSkillCreation::default()
            },
        )
        .await
        .unwrap();
        let write = created_write(&repository);
        assert_eq!(write.file_name.as_deref(), Some("first.zip"));
        assert_eq!(write.file_content.as_deref(), Some("Zmlyc3Q="));
        assert_eq!(write.file_type.as_deref(), Some("application/zip"));
    });
}

#[test]
fn create_keeps_explicit_bytes_over_the_uploaded_tuples() {
    support::runtime().block_on(async {
        let repository = ScriptedRepository::default();
        let ops = support::ops(repository.clone());
        let mut logging = support::logging_state();
        let files = uploads();
        ops.create_skill(
            &mut logging,
            HostedSkillCreation {
                files: Some(&files),
                file_content: Some(b"explicit"),
                file_name: Some("explicit.zip"),
                file_type: Some("application/x-explicit"),
                principal: Some(&principal("alice")),
                ..HostedSkillCreation::default()
            },
        )
        .await
        .unwrap();
        let write = created_write(&repository);
        assert_eq!(write.file_name.as_deref(), Some("explicit.zip"));
        assert_eq!(write.file_content.as_deref(), Some("ZXhwbGljaXQ="));
        assert_eq!(write.file_type.as_deref(), Some("application/x-explicit"));
    });
}

#[test]
fn create_lets_the_tuples_replace_empty_explicit_bytes() {
    support::runtime().block_on(async {
        let repository = ScriptedRepository::default();
        let ops = support::ops(repository.clone());
        let mut logging = support::logging_state();
        let files = uploads();
        ops.create_skill(
            &mut logging,
            HostedSkillCreation {
                files: Some(&files),
                file_content: Some(b""),
                file_name: Some("explicit.zip"),
                file_type: Some("application/x-explicit"),
                principal: Some(&principal("alice")),
                ..HostedSkillCreation::default()
            },
        )
        .await
        .unwrap();
        let write = created_write(&repository);
        assert_eq!(write.file_name.as_deref(), Some("first.zip"));
        assert_eq!(write.file_content.as_deref(), Some("Zmlyc3Q="));
        assert_eq!(write.file_type.as_deref(), Some("application/zip"));
    });
}

#[test]
fn create_keeps_a_null_mime_type_from_a_three_element_tuple() {
    support::runtime().block_on(async {
        let repository = ScriptedRepository::default();
        let ops = support::ops(repository.clone());
        let mut logging = support::logging_state();
        let files = [support::upload(
            "first.zip",
            b"first",
            SkillUploadType::Given(None),
        )];
        ops.create_skill(
            &mut logging,
            HostedSkillCreation {
                files: Some(&files),
                file_name: Some("explicit.zip"),
                file_type: Some("application/x-explicit"),
                principal: Some(&principal("alice")),
                ..HostedSkillCreation::default()
            },
        )
        .await
        .unwrap();
        let write = created_write(&repository);
        assert_eq!(write.file_name.as_deref(), Some("first.zip"));
        assert_eq!(write.file_type, None);
    });
}

#[test]
fn create_ignores_an_empty_tuple_list() {
    support::runtime().block_on(async {
        let repository = ScriptedRepository::default();
        let ops = support::ops(repository.clone());
        let mut logging = support::logging_state();
        let files: [SkillUpload<'_>; 0] = [];
        ops.create_skill(
            &mut logging,
            HostedSkillCreation {
                files: Some(&files),
                file_name: Some("explicit.zip"),
                file_type: Some("application/x-explicit"),
                principal: Some(&principal("alice")),
                ..HostedSkillCreation::default()
            },
        )
        .await
        .unwrap();
        let write = created_write(&repository);
        assert_eq!(write.file_name.as_deref(), Some("explicit.zip"));
        assert_eq!(write.file_type.as_deref(), Some("application/x-explicit"));
    });
}

#[test]
fn create_updates_the_call_context_before_the_store_fails() {
    support::runtime().block_on(async {
        let repository = ScriptedRepository::default();
        repository.update(|state| state.create_failure = true);
        let ops = support::ops(repository.clone());
        let mut logging = support::logging_state();
        let outcome = ops
            .create_skill(
                &mut logging,
                HostedSkillCreation {
                    display_title: Some("Title"),
                    principal: Some(&principal("alice")),
                    ..HostedSkillCreation::default()
                },
            )
            .await;
        assert!(matches!(
            outcome,
            Err(HostedSkillOpsError::Store(StoreError::Repository { .. }))
        ));
        assert_eq!(
            logged(&logging, "optional_params"),
            object(&[("display_title", support::text("Title"))])
        );
    });
}

#[test]
fn create_rejects_a_caller_without_identity_after_logging() {
    support::runtime().block_on(async {
        let ops = support::ops(ScriptedRepository::default());
        let mut logging = support::logging_state();
        let outcome = ops
            .create_skill(&mut logging, HostedSkillCreation::default())
            .await;
        assert!(matches!(
            outcome,
            Err(HostedSkillOpsError::Store(StoreError::MissingIdentity))
        ));
        assert_eq!(
            logged(&logging, "optional_params"),
            object(&[("display_title", Json::Null)])
        );
    });
}

#[test]
fn list_forwards_the_page_and_the_caller_scopes() {
    support::runtime().block_on(async {
        let repository = ScriptedRepository::default();
        let ops = support::ops(repository.clone());
        let mut logging = support::logging_state();
        let page = SkillPage {
            limit: 2,
            offset: 5,
        };
        let response = ops
            .list_skills(
                &mut logging,
                page,
                Some(&principal("alice")),
                Some("call-9"),
            )
            .await
            .unwrap();
        assert!(response.data.is_empty());
        assert_eq!(
            repository.effects(),
            vec![Effect::FindMany {
                page,
                created_by: Some(
                    [
                        "alice",
                        "user:alice",
                        "team:team-1",
                        "org:org-1",
                        "key:key-1",
                        "key:token-1"
                    ]
                    .map(str::to_owned)
                    .to_vec()
                ),
            }]
        );
        assert_eq!(
            logged(&logging, "optional_params"),
            object(&[("limit", number(2)), ("offset", number(5))])
        );
    });
}

#[test]
fn list_forwards_an_admin_as_an_unscoped_page() {
    support::runtime().block_on(async {
        let repository = ScriptedRepository::default();
        let ops = support::ops(repository.clone());
        let mut logging = support::logging_state();
        let page = SkillPage {
            limit: 1,
            offset: 0,
        };
        ops.list_skills(&mut logging, page, Some(&admin("root")), None)
            .await
            .unwrap();
        assert_eq!(
            repository.effects(),
            vec![Effect::FindMany {
                page,
                created_by: None,
            }]
        );
    });
}

#[test]
fn get_forwards_the_principal_to_the_store() {
    support::runtime().block_on(async {
        let repository = ScriptedRepository::with_row(support::stored_row("alice"));
        let ops = support::ops(repository.clone());
        let mut logging = support::logging_state();
        let outcome = ops
            .get_skill(
                &mut logging,
                support::SKILL_ID,
                Some(&principal("bob")),
                None,
            )
            .await;
        assert!(matches!(
            outcome,
            Err(HostedSkillOpsError::Store(StoreError::NotFound { .. }))
        ));
        assert_eq!(
            repository.effects(),
            vec![Effect::FindUnique(support::SKILL_ID.to_owned())]
        );
    });
}

#[test]
fn delete_updates_the_call_context_before_the_store() {
    support::runtime().block_on(async {
        let repository = ScriptedRepository::with_row(support::stored_row("alice"));
        let ops = support::ops(repository.clone());
        let mut logging = support::logging_state();
        let deleted = ops
            .delete_skill(&mut logging, support::SKILL_ID, None, Some("call-9"))
            .await
            .unwrap();
        assert_eq!(deleted.id, support::SKILL_ID);
        assert_eq!(
            repository.effects(),
            vec![
                Effect::FindUnique(support::SKILL_ID.to_owned()),
                Effect::Delete(support::SKILL_ID.to_owned()),
            ]
        );
        assert_eq!(
            logged(&logging, "optional_params"),
            object(&[("skill_id", support::text(support::SKILL_ID))])
        );
    });
}
