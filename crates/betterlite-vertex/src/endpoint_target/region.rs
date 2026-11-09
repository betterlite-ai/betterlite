use betterlite_models::catalog::ModelCatalog;

pub const DEFAULT_LOCATION: &str = "us-central1";

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct RegionSelection {
    pub region: String,
    pub unsupported: Option<UnsupportedRegion>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct UnsupportedRegion {
    pub configured: String,
    pub supported: Vec<String>,
}

#[must_use]
pub fn select_region(
    catalog: &ModelCatalog,
    model: &str,
    configured: Option<&str>,
) -> RegionSelection {
    let supported = supported_regions(catalog, model);
    let Some(first) = supported.first() else {
        let region = configured.map_or_else(|| DEFAULT_LOCATION.to_owned(), str::to_owned);
        return RegionSelection {
            region,
            unsupported: None,
        };
    };
    let Some(region) = configured else {
        return RegionSelection {
            region: first.clone(),
            unsupported: None,
        };
    };
    if supported.iter().any(|known| known == region) {
        return RegionSelection {
            region: region.to_owned(),
            unsupported: None,
        };
    }
    // LiteLLM routes an unsupported configured region to the model's first supported one and warns
    tracing::warn!(
        model,
        configured = region,
        selected = %first,
        "vertex model does not support the configured region"
    );
    RegionSelection {
        region: first.clone(),
        unsupported: Some(UnsupportedRegion {
            configured: region.to_owned(),
            supported: supported.to_vec(),
        }),
    }
}

fn supported_regions<'catalog>(catalog: &'catalog ModelCatalog, model: &str) -> &'catalog [String] {
    let key = if model.starts_with("vertex_ai/") {
        model.to_owned()
    } else {
        format!("vertex_ai/{model}")
    };
    catalog
        .lookup
        .get(&key)
        .and_then(|canonical| catalog.entries.get(canonical.as_ref()))
        .map_or(&[], |entry| {
            entry.fields.metadata.supported_regions.as_slice()
        })
}
