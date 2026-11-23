mod tests;

use betterlite_db::search_tools::convert::isoformat;
use betterlite_models::decisions::Json;

use super::error::ClaudePluginReadFailure;
use super::{
    AUTHOR_FIELD, CATEGORY_FIELD, DOMAIN_FIELD, HOMEPAGE_FIELD, KEYWORDS_FIELD, NAMESPACE_FIELD,
    PluginDetail, PluginListAuthor, PluginListItem, PluginManifestFields, PluginRow, SOURCE_FIELD,
};
use crate::api::claude_plugin_policy::source::PluginSource;

struct ListFields {
    source: PluginSource,
    author: Option<PluginListAuthor>,
    homepage: Option<String>,
    keywords: Option<Vec<String>>,
    category: Option<String>,
    domain: Option<String>,
    namespace: Option<String>,
}

pub(super) fn list_item(row: PluginRow) -> Result<PluginListItem, ClaudePluginReadFailure> {
    let PluginRow {
        id,
        name,
        version,
        description,
        enabled,
        created_at,
        updated_at,
        manifest,
        ..
    } = row;
    let fields = list_fields(manifest)?;
    Ok(PluginListItem {
        id,
        name,
        version,
        description,
        source: fields.source,
        author: fields.author,
        homepage: fields.homepage,
        keywords: fields.keywords,
        category: fields.category,
        domain: fields.domain,
        namespace: fields.namespace,
        enabled,
        created_at: created_at.map(isoformat),
        updated_at: updated_at.map(isoformat),
    })
}

fn list_fields(manifest: PluginManifestFields) -> Result<ListFields, ClaudePluginReadFailure> {
    let source = match manifest.source {
        None => PluginSource::default(),
        Some(value) => plugin_source(value)?,
    };
    let author = match manifest.author {
        None | Some(Json::Null) => None,
        Some(value) => Some(plugin_author(value)?),
    };
    let homepage = manifest
        .homepage
        .map_or(Ok(None), |value| optional_text(HOMEPAGE_FIELD, value))?;
    let keywords = manifest
        .keywords
        .map_or(Ok(None), |value| string_list(KEYWORDS_FIELD, value))?;
    let category = manifest
        .category
        .map_or(Ok(None), |value| optional_text(CATEGORY_FIELD, value))?;
    let domain = manifest
        .domain
        .map_or(Ok(None), |value| optional_text(DOMAIN_FIELD, value))?;
    let namespace = manifest
        .namespace
        .map_or(Ok(None), |value| optional_text(NAMESPACE_FIELD, value))?;
    Ok(ListFields {
        source,
        author,
        homepage,
        keywords,
        category,
        domain,
        namespace,
    })
}

pub(super) fn detail(row: PluginRow) -> PluginDetail {
    let PluginRow {
        id,
        name,
        version,
        description,
        enabled,
        created_at,
        updated_at,
        created_by,
        manifest,
    } = row;
    PluginDetail {
        id,
        name,
        version,
        description,
        source: manifest.source.unwrap_or(Json::Null),
        author: manifest.author.unwrap_or(Json::Null),
        homepage: manifest.homepage.unwrap_or(Json::Null),
        keywords: manifest.keywords.unwrap_or(Json::Null),
        category: manifest.category.unwrap_or(Json::Null),
        enabled,
        created_at: created_at.map(isoformat),
        updated_at: updated_at.map(isoformat),
        created_by,
    }
}

// dict[str, str]: every value must be a string, and a key outside the known source fields is kept
fn plugin_source(value: Json) -> Result<PluginSource, ClaudePluginReadFailure> {
    let Json::Object(fields) = value else {
        return Err(invalid_field(SOURCE_FIELD));
    };
    let mut source = PluginSource::default();
    for (field_name, field_value) in fields {
        let Json::Text(field_text) = field_value else {
            return Err(invalid_field(SOURCE_FIELD));
        };
        match field_name.as_str() {
            "source" => source.source = Some(field_text),
            "repo" => source.repo = Some(field_text),
            "url" => source.url = Some(field_text),
            "path" => source.path = Some(field_text),
            "sha256" => source.sha256 = Some(field_text),
            _ => {
                source.extra.insert(field_name, field_text);
            }
        }
    }
    Ok(source)
}

// PluginAuthor: name is required, email is an optional string, and an unknown key is ignored
fn plugin_author(value: Json) -> Result<PluginListAuthor, ClaudePluginReadFailure> {
    let Json::Object(mut fields) = value else {
        return Err(invalid_field(AUTHOR_FIELD));
    };
    let Some(name_value) = fields.shift_remove("name") else {
        return Err(invalid_field(AUTHOR_FIELD));
    };
    let Json::Text(name) = name_value else {
        return Err(invalid_field(AUTHOR_FIELD));
    };
    let email = match fields.shift_remove("email") {
        None | Some(Json::Null) => None,
        Some(Json::Text(text)) => Some(text),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            return Err(invalid_field(AUTHOR_FIELD));
        }
    };
    Ok(PluginListAuthor { name, email })
}

fn optional_text(
    field: &'static str,
    value: Json,
) -> Result<Option<String>, ClaudePluginReadFailure> {
    match value {
        Json::Null => Ok(None),
        Json::Text(text) => Ok(Some(text)),
        Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            Err(invalid_field(field))
        }
    }
}

fn string_list(
    field: &'static str,
    value: Json,
) -> Result<Option<Vec<String>>, ClaudePluginReadFailure> {
    match value {
        Json::Null => Ok(None),
        Json::List(items) => {
            let mut texts = Vec::with_capacity(items.len());
            for item in items {
                let Json::Text(text) = item else {
                    return Err(invalid_field(field));
                };
                texts.push(text);
            }
            Ok(Some(texts))
        }
        Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => {
            Err(invalid_field(field))
        }
    }
}

fn invalid_field(field: &'static str) -> ClaudePluginReadFailure {
    ClaudePluginReadFailure::Manifest {
        message: format!("the stored plugin manifest field '{field}' has an unexpected value"),
    }
}
