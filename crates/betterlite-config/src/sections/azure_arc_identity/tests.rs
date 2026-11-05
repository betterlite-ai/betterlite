#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::ConfigLoader;
use crate::sections::azure_arc_identity::Settings;

fn settings(variables: &[(&str, &str)]) -> Settings {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load("")
        .unwrap()
        .sections
        .azure_arc_identity
}

#[rstest]
#[case::loads_programdata("PROGRAMDATA", "D:\\PD", |settings: &Settings| settings.program_data.clone(), |settings: &Settings| settings.program_files.clone())]
#[case::loads_programfiles("ProgramFiles", "D:\\Files", |settings: &Settings| settings.program_files.clone(), |settings: &Settings| settings.program_data.clone())]
fn azure_arc_identity_config_cases(
    #[case] name: &str,
    #[case] value: &str,
    #[case] read: fn(&Settings) -> Option<String>,
    #[case] other: fn(&Settings) -> Option<String>,
) {
    let explicit = settings(&[(name, value)]);
    assert_eq!(read(&explicit).as_deref(), Some(value));
    assert_eq!(other(&explicit), None);
    assert_eq!(read(&settings(&[])), None);
    assert_eq!(read(&settings(&[(name, "")])), None);
}
