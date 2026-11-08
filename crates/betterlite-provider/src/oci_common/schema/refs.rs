use std::collections::BTreeSet;

use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::definitions::estimate::estimate_json_bytes;
use betterlite_prompts::definitions::legacy::DEFS_MAX_INLINED_BYTES;

use crate::oci_common::OciError;

const MAX_SCHEMA_DEPTH: usize = 256;

struct Walk<'defs> {
    defs: Option<&'defs Object>,
    resolving: BTreeSet<String>,
    spent: usize,
}

pub fn resolve_oci_schema_refs(schema: &Json) -> Result<Json, OciError> {
    // the table is dropped from the result anyway, so only definitions a reference reaches are walked and charged
    let mut root = schema.clone();
    let defs = match &mut root {
        Json::Object(object) => object.shift_remove("$defs"),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    };
    let mut walk = Walk {
        defs: defs.as_ref().and_then(Json::as_object),
        resolving: BTreeSet::new(),
        spent: 0,
    };
    let mut resolved = resolve_refs(&root, &mut walk, 0)?;
    if let Json::Object(fields) = &mut resolved {
        fields.shift_remove("$defs");
    }
    Ok(resolved)
}

fn resolve_refs(value: &Json, walk: &mut Walk<'_>, depth: usize) -> Result<Json, OciError> {
    if depth >= MAX_SCHEMA_DEPTH {
        return Ok(Json::Object(Object::from([(
            "type".to_owned(),
            Json::Text("object".to_owned()),
        )])));
    }
    let next = depth.saturating_add(1);
    let fields = match value {
        Json::Object(fields) => fields,
        Json::List(items) => return resolve_list(items, walk, next),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => return Ok(value.clone()),
    };
    match fields.get("$ref") {
        None | Some(Json::Null) => {}
        Some(Json::Text(reference)) if reference.starts_with("#/$defs/") => {
            let resolved = resolve_reference(value, reference, walk, depth)?;
            return Ok(resolved.unwrap_or_else(|| {
                Json::Object(Object::from([(
                    "type".to_owned(),
                    Json::Text("object".to_owned()),
                )]))
            }));
        }
        Some(_) => return Ok(value.clone()),
    }
    resolve_fields(fields, walk, next)
}

fn resolve_list(items: &[Json], walk: &mut Walk<'_>, next: usize) -> Result<Json, OciError> {
    let resolved = items
        .iter()
        .map(|item| resolve_refs(item, walk, next))
        .collect::<Result<Vec<Json>, OciError>>()?;
    Ok(Json::List(resolved))
}

fn resolve_fields(fields: &Object, walk: &mut Walk<'_>, next: usize) -> Result<Json, OciError> {
    let mut resolved = Object::new();
    for (key, field) in fields {
        resolved.insert(key.clone(), resolve_refs(field, walk, next)?);
    }
    Ok(Json::Object(resolved))
}

fn resolve_reference(
    value: &Json,
    reference: &str,
    walk: &mut Walk<'_>,
    depth: usize,
) -> Result<Option<Json>, OciError> {
    let key = reference.rsplit('/').next().unwrap_or_default().to_owned();
    if !walk.resolving.insert(key.clone()) {
        return Ok(None);
    }
    let target = walk.defs.and_then(|table| table.get(&key)).unwrap_or(value);
    walk.spent = walk.spent.saturating_add(estimate_json_bytes(target));
    if walk.spent > DEFS_MAX_INLINED_BYTES {
        return Err(OciError {
            status: 400,
            message: format!(
                "the OCI tool schema expands past {DEFS_MAX_INLINED_BYTES} estimated bytes"
            ),
        });
    }
    let resolved = resolve_refs(target, walk, depth.saturating_add(1));
    walk.resolving.remove(&key);
    resolved.map(Some)
}
