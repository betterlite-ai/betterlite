#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::ConfigLoader;
use crate::sections::azure_client_secret::Settings;

fn settings(variables: &[(&str, &str)]) -> Settings {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load("")
        .unwrap()
        .sections
        .azure_client_secret
}

#[rstest]
#[case::loads_azure_regional_authority_name("AZURE_REGIONAL_AUTHORITY_NAME", None, |settings: &Settings| settings.azure_regional_authority_name.clone())]
#[case::loads_msal_force_region("MSAL_FORCE_REGION", None, |settings: &Settings| settings.msal_force_region.clone())]
#[case::loads_region_name("REGION_NAME", Some(""), |settings: &Settings| Some(settings.region_name.clone()))]
fn azure_identity_region_config_cases(
    #[case] name: &str,
    #[case] unset: Option<&str>,
    #[case] read: fn(&Settings) -> Option<String>,
) {
    let explicit = settings(&[(name, "configured")]);
    assert_eq!(read(&explicit).as_deref(), Some("configured"));
    assert_eq!(read(&settings(&[])).as_deref(), unset);
    assert_eq!(read(&settings(&[(name, "")])).as_deref(), Some(""));
    assert_eq!(read(&settings(&[("UNRELATED", "value")])).as_deref(), unset);
    assert_eq!(
        read(&settings(&[
            ("AZURE_REGION", "eastus"),
            ("REGION", "eastus")
        ]))
        .as_deref(),
        unset
    );
}
