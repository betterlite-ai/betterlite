use std::collections::{BTreeMap, BTreeSet};

use betterlite_auth::auth_model_lists::credential::CredentialSnapshot;
use betterlite_core::api::response_id_traversal::reasoning::strip_encrypted_reasoning as strip_input_reasoning;
use betterlite_core::api::response_id_traversal::record::Record;
use betterlite_prompts::message_content::{Content, ContentPart, Message};
use betterlite_prompts::reasoning_fields::encrypted_signature::encrypted_content_of_block;
use betterlite_prompts::reasoning_fields::strip_encrypted::strip_encrypted_reasoning as strip_message_blocks;

use crate::deployment_identity::spec::DeploymentSpec;
use crate::model_catalog_lookup::lookup::CatalogLookup;
use crate::responses_affinity::boundary::{Boundary, encryption_boundary};
use crate::responses_affinity::error::AffinityError;
use crate::responses_affinity::history::History;
use crate::responses_affinity::identity::deployment_id;
use crate::responses_affinity::markers::{item_model_id, wrapped_model_id};

// _strip_reasoning_the_target_cannot_decrypt: drop the marked history none of the targets can read, in place.
pub(crate) fn strip_for_targets(
    history: &mut History<'_>,
    targets: &[DeploymentSpec],
    router: Option<&CatalogLookup<'_>>,
    credentials: &CredentialSnapshot,
) -> Result<(), AffinityError> {
    let target_ids: BTreeSet<String> = targets.iter().filter_map(deployment_id).collect();
    let target_boundaries: BTreeSet<Boundary> = targets
        .iter()
        .filter_map(|target| encryption_boundary(&target.litellm_params, credentials))
        .collect();
    let origins = decryptable_origins(
        history,
        &target_ids,
        &target_boundaries,
        router,
        credentials,
    )?;
    let item_predicate = |item: &Record| {
        item_model_id(item).is_some_and(|origin| origins.get(&origin).copied() != Some(true))
    };
    let item_selected: Option<&dyn Fn(&Record) -> bool> = Some(&item_predicate);
    if let Some(input) = history.input.as_deref_mut() {
        strip_input_reasoning(input, item_selected);
    }
    let block_predicate = |block: &ContentPart| {
        encrypted_content_of_block(block)
            .as_deref()
            .and_then(wrapped_model_id)
            .is_some_and(|origin| origins.get(&origin).copied() != Some(true))
    };
    strip_messages(history, Some(&block_predicate));
    Ok(())
}

// The fallback path strips every bridged reason: no configured deployment can decrypt any of it.
pub(crate) fn strip_all(history: &mut History<'_>) {
    if let Some(input) = history.input.as_deref_mut() {
        strip_input_reasoning(input, None);
    }
    strip_messages(history, None);
}

// target_can_decrypt: a target id reads its own reasoning, and an origin on a target's boundary is readable too.
fn decryptable_origins(
    history: &History<'_>,
    target_ids: &BTreeSet<String>,
    target_boundaries: &BTreeSet<Boundary>,
    router: Option<&CatalogLookup<'_>>,
    credentials: &CredentialSnapshot,
) -> Result<BTreeMap<String, bool>, AffinityError> {
    let mut origins = BTreeSet::new();
    if let Some(Record::List(items)) = history.input.as_deref() {
        origins.extend(items.iter().filter_map(item_model_id));
    }
    if let Some(messages) = history.effective_messages() {
        add_message_origins(messages, &mut origins);
    }
    let mut decryptable = BTreeMap::new();
    for origin in origins {
        let readable = if target_ids.contains(&origin) {
            true
        } else {
            origin_decryptable(&origin, target_boundaries, router, credentials)?
        };
        decryptable.insert(origin, readable);
    }
    Ok(decryptable)
}

fn add_message_origins(messages: &[Message], origins: &mut BTreeSet<String>) {
    for message in messages {
        let Some(Content::Parts(blocks)) = &message.content else {
            continue;
        };
        origins.extend(blocks.iter().filter_map(|block| {
            encrypted_content_of_block(block)
                .as_deref()
                .and_then(wrapped_model_id)
        }));
    }
}

// The catalog half of target_can_decrypt, resolved once per distinct origin.
fn origin_decryptable(
    origin: &str,
    target_boundaries: &BTreeSet<Boundary>,
    router: Option<&CatalogLookup<'_>>,
    credentials: &CredentialSnapshot,
) -> Result<bool, AffinityError> {
    let Some(catalog) = router else {
        return Ok(false);
    };
    let Some(deployment) = catalog.get_deployment(origin)? else {
        return Ok(false);
    };
    let boundary = encryption_boundary(&deployment.litellm_params, credentials);
    Ok(boundary.is_some_and(|candidate| target_boundaries.contains(&candidate)))
}

fn strip_messages(history: &mut History<'_>, predicate: Option<&dyn Fn(&ContentPart) -> bool>) {
    let Some(messages) = history.effective_messages_mut() else {
        return;
    };
    *messages = strip_message_blocks(messages, predicate);
}
