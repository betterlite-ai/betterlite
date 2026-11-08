use crate::otel::config::{ExporterOwner, ExporterSpec};

#[must_use]
pub fn ensure_mappers(names: &[String], extra: &[&str]) -> Vec<String> {
    let mut result = names.to_vec();
    for name in extra {
        if !result.iter().any(|present| present == name) {
            result.push((*name).to_owned());
        }
    }
    result
}

#[must_use]
pub fn credential_gated_exporters(
    exporters: Vec<ExporterSpec>,
    owner: ExporterOwner,
) -> Vec<ExporterSpec> {
    let mut gated: Vec<ExporterSpec> = exporters
        .into_iter()
        .filter(|spec| !is_unconfigured_placeholder(spec))
        .collect();
    gated.push(ExporterSpec {
        kind: "console".to_owned(),
        endpoint: None,
        traces_endpoint: None,
        headers: None,
        owner: Some(owner),
        options: None,
        use_simple_processor: None,
        requires_headers: true,
    });
    gated
}

#[must_use]
pub fn is_unconfigured_placeholder(spec: &ExporterSpec) -> bool {
    spec.kind == "console"
        && spec.endpoint.is_none()
        && spec.traces_endpoint.is_none()
        && spec.headers.is_none()
        && spec.owner.is_none()
        && spec.options.is_none()
        && spec.use_simple_processor.is_none()
        && !spec.requires_headers
}

mod tests;
