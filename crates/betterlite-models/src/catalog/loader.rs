use std::collections::BTreeMap;

use indexmap::IndexMap;
use serde_json::value::RawValue;

use super::{
    CatalogEntry, CatalogError, FALLBACK_GENERALIZATIONS, ModelCatalog, SAMPLE_SPEC, UnknownFields,
};
use crate::catalog_generalizations::{CatalogGeneralizations, decode_generalization_rules};
use crate::entry::{ModelFieldError, ModelFields};
use crate::price_fields::PriceFields;

pub(super) fn build_catalog(
    raw: IndexMap<String, Box<RawValue>>,
    unknown: UnknownFields,
) -> Result<ModelCatalog, CatalogError> {
    let mut entries = BTreeMap::new();
    let mut lookup = BTreeMap::new();
    let mut warnings = Vec::new();
    let (mut generalizations, mut alias_claims) = (CatalogGeneralizations::default(), Vec::new());
    for (name, value) in raw {
        if name == SAMPLE_SPEC {
            continue;
        }
        if name == FALLBACK_GENERALIZATIONS {
            generalizations = install_generalizations(value.as_ref(), unknown, &mut warnings)?;
            continue;
        }
        let fields_map: BTreeMap<String, Box<RawValue>> = serde_json::from_str(value.get())?;
        let typed_value = without_aliases(&fields_map)?;
        let fields = parse_fields(&name, typed_value.as_ref(), unknown, &mut warnings)?;
        let prices =
            PriceFields::parse(typed_value.as_ref()).map_err(|source| CatalogError::Prices {
                model: name.clone(),
                source,
            })?;
        entries.insert(
            name.clone(),
            CatalogEntry {
                name: name.clone(),
                fields,
                prices,
            },
        );
        insert_lookup(&mut lookup, &name, &name, &mut warnings);
        add_alias_claims(&name, &fields_map, &mut warnings, &mut alias_claims);
    }
    for (alias, canonical) in alias_claims {
        insert_lookup(&mut lookup, &alias, &canonical, &mut warnings);
    }
    Ok(ModelCatalog {
        entries,
        lookup,
        generalizations,
        warnings,
    })
}

fn add_alias_claims(
    name: &str,
    fields: &BTreeMap<String, Box<RawValue>>,
    warnings: &mut Vec<String>,
    claims: &mut Vec<(String, String)>,
) {
    let aliases = aliases(name, fields, warnings);
    claims.extend(aliases.into_iter().map(|alias| (alias, name.to_owned())));
}

fn install_generalizations(
    value: &RawValue,
    unknown: UnknownFields,
    warnings: &mut Vec<String>,
) -> Result<CatalogGeneralizations, CatalogError> {
    let Ok(declaration) = serde_json::from_str::<BTreeMap<String, Box<RawValue>>>(value.get())
    else {
        return Ok(CatalogGeneralizations::default());
    };
    let Some(rules) = declaration.get("rules") else {
        return Ok(CatalogGeneralizations::default());
    };
    let decoded = decode_generalization_rules(rules.get().as_bytes())?;
    for (index, rule) in decoded.iter().enumerate() {
        validate_model_info(rule, index, unknown, warnings)?;
    }
    let (installed, install_warnings) = CatalogGeneralizations::install(decoded);
    warnings.extend(install_warnings.into_iter().map(|error| error.to_string()));
    Ok(installed)
}

fn validate_model_info(
    rule: &RawValue,
    index: usize,
    unknown: UnknownFields,
    warnings: &mut Vec<String>,
) -> Result<(), CatalogError> {
    let Ok(rule_fields) = serde_json::from_str::<BTreeMap<String, Box<RawValue>>>(rule.get())
    else {
        return Ok(());
    };
    let Some(fields) = rule_fields.get("model_info") else {
        return Ok(());
    };
    let path = format!("fallback_generalizations.rules[{index}].model_info");
    match parse_fields(&path, fields.as_ref(), unknown, warnings) {
        Err(
            error @ CatalogError::Fields {
                source: ModelFieldError::Unknown(_),
                ..
            },
        ) => Err(error),
        Ok(_) | Err(CatalogError::Fields { .. }) => Ok(()),
        Err(error) => Err(error),
    }
}

fn parse_fields(
    name: &str,
    value: &RawValue,
    unknown: UnknownFields,
    warnings: &mut Vec<String>,
) -> Result<ModelFields, CatalogError> {
    let parsed = match unknown {
        UnknownFields::Reject => ModelFields::parse_strict(value),
        UnknownFields::Warn => {
            let (fields, paths) =
                ModelFields::parse(value).map_err(|source| CatalogError::Fields {
                    model: name.to_owned(),
                    source,
                })?;
            warnings.extend(paths.into_iter().map(|path| format!("{name}.{path}")));
            Ok(fields)
        }
    };
    parsed.map_err(|source| CatalogError::Fields {
        model: name.to_owned(),
        source,
    })
}

fn without_aliases(
    fields: &BTreeMap<String, Box<RawValue>>,
) -> Result<Box<RawValue>, CatalogError> {
    let typed_fields = fields
        .iter()
        .filter(|(key, _)| key.as_str() != "aliases")
        .map(|(key, value)| (key.clone(), value))
        .collect::<BTreeMap<_, _>>();
    Ok(serde_json::from_slice(&serde_json::to_vec(&typed_fields)?)?)
}

fn aliases(
    model: &str,
    fields: &BTreeMap<String, Box<RawValue>>,
    warnings: &mut Vec<String>,
) -> Vec<String> {
    let Some(raw) = fields.get("aliases") else {
        return Vec::new();
    };
    match serde_json::from_str::<Vec<String>>(raw.get()) {
        Ok(values) => values,
        Err(error) => {
            warnings.push(format!("{model}.aliases: {error}"));
            Vec::new()
        }
    }
}

fn insert_lookup(
    lookup: &mut BTreeMap<String, String>,
    key: &str,
    canonical: &str,
    warnings: &mut Vec<String>,
) {
    if let Some(previous) = lookup.get(key) {
        warnings.push(format!(
            "alias collision for {key}: kept {previous}, ignored {canonical}"
        ));
    } else {
        lookup.insert(key.to_owned(), canonical.to_owned());
    }
}
