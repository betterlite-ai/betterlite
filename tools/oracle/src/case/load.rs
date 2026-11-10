use std::fs;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;

use crate::OracleError;
use crate::http_scenarios::decode_http_scenario;
use crate::http_scenarios::model::{HttpScenario, Limits};
use crate::routing_component_oracle::{ProbeCase, validate_probe_case};
use crate::sdk_http::{OracleLimits as SdkHttpLimits, SdkHttpCase, decode_sdk_http_case};

use super::{CaseKind, LoadedCase, parse_case};

fn load_selected<T, F>(
    paths: &[PathBuf],
    keep: fn(Option<&str>) -> bool,
    decode: F,
) -> Result<Vec<T>, OracleError>
where
    F: Fn(&Path, &str) -> Result<T, OracleError>,
{
    let mut cases = Vec::new();
    for (path, text) in read_case_files(paths)? {
        let probe: CaseKind = parse_case_text(&path, &text)?;
        if keep(probe.kind.as_deref()) {
            cases.push(decode(&path, &text)?);
        }
    }
    Ok(cases)
}

// sdk_http, probe and http_scenario cases run through their own drivers, so this list leaves them out
pub(crate) fn load_cases(paths: &[PathBuf]) -> Result<Vec<LoadedCase>, OracleError> {
    load_selected(
        paths,
        |kind| !matches!(kind, Some("sdk_http" | "probe" | "http_scenario")),
        parse_case,
    )
}

pub(crate) fn load_http_scenario_cases(
    paths: &[PathBuf],
    limits: &Limits,
) -> Result<Vec<HttpScenario>, OracleError> {
    load_selected(
        paths,
        |kind| kind == Some("http_scenario"),
        |path, text| {
            decode_http_scenario(text.as_bytes(), limits).map_err(|source| {
                OracleError::HttpScenarioCase {
                    path: path.to_owned(),
                    source: Box::new(source),
                }
            })
        },
    )
}

pub(crate) fn load_sdk_http_cases(paths: &[PathBuf]) -> Result<Vec<SdkHttpCase>, OracleError> {
    cases_matching(paths, |kind| kind == Some("sdk_http"))?
        .iter()
        .map(|(path, text)| decode_sdk_case(path, text))
        .collect()
}

pub(crate) fn load_probe_cases(paths: &[PathBuf]) -> Result<Vec<ProbeCase>, OracleError> {
    cases_matching(paths, |kind| kind == Some("probe"))?
        .iter()
        .map(|(path, text)| decode_probe_case(path, text))
        .collect()
}

fn decode_sdk_case(path: &Path, text: &str) -> Result<SdkHttpCase, OracleError> {
    let case =
        decode_sdk_http_case(text.as_bytes(), &SdkHttpLimits::default()).map_err(|source| {
            OracleError::SdkHttpCase {
                path: path.to_owned(),
                source,
            }
        })?;
    if case.compare.contracts.is_some() {
        return Err(OracleError::CaseInvalid {
            path: path.to_owned(),
            message:
                "compare.numeric and compare.expected_difference are not applied to sdk_http cases"
                    .to_owned(),
        });
    }
    Ok(case)
}

fn decode_probe_case(path: &Path, text: &str) -> Result<ProbeCase, OracleError> {
    let case: ProbeCase = parse_case_text(path, text)?;
    validate_probe_case(path, &case)?;
    Ok(case)
}

pub(super) fn parse_case_text<T>(path: &Path, text: &str) -> Result<T, OracleError>
where
    T: DeserializeOwned,
{
    serde_saphyr::from_str(text).map_err(|source| OracleError::CaseParse {
        path: path.to_owned(),
        source: Box::new(source),
    })
}

fn cases_matching<F>(paths: &[PathBuf], matches: F) -> Result<Vec<(PathBuf, String)>, OracleError>
where
    F: Fn(Option<&str>) -> bool,
{
    let mut selected = Vec::new();
    for (path, text) in read_case_files(paths)? {
        let probe: CaseKind = parse_case_text(&path, &text)?;
        if matches(probe.kind.as_deref()) {
            selected.push((path, text));
        }
    }
    Ok(selected)
}

fn read_case_files(paths: &[PathBuf]) -> Result<Vec<(PathBuf, String)>, OracleError> {
    let mut files = Vec::new();
    for path in paths {
        collect_yaml(path, &mut files)?;
    }
    if files.is_empty() {
        return Err(OracleError::CaseInvalid {
            path: PathBuf::from("<arguments>"),
            message: "no YAML cases found".to_owned(),
        });
    }
    files.sort();
    let mut texts = Vec::with_capacity(files.len());
    for path in files {
        let text = fs::read_to_string(&path).map_err(|source| OracleError::CaseRead {
            path: path.clone(),
            source,
        })?;
        texts.push((path, text));
    }
    Ok(texts)
}

fn collect_yaml(path: &Path, files: &mut Vec<PathBuf>) -> Result<(), OracleError> {
    let read_error = |source| OracleError::CaseRead {
        path: path.to_owned(),
        source,
    };
    let metadata = fs::metadata(path).map_err(&read_error)?;
    if metadata.is_file() {
        if path
            .extension()
            .is_some_and(|extension| extension == "yaml" || extension == "yml")
        {
            files.push(path.to_owned());
        }
        return Ok(());
    }
    let entries = fs::read_dir(path).map_err(&read_error)?;
    for result in entries {
        let entry = result.map_err(&read_error)?;
        collect_yaml(&entry.path(), files)?;
    }
    Ok(())
}
