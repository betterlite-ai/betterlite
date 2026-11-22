use std::fs;
use std::path::PathBuf;
use std::str;

use betterlite_db::team_rows::aliases::ModelAliases;
use rstest::rstest;

use crate::identity_grants::team_grants::{ModelAliasMap, decode_model_aliases};

// Stored text reads as None unless it is a text-to-text object; a mapping keeps every alias or none.
#[rstest]
fn replays_the_team_model_aliases_corpus(
    #[files("fuzz/corpus/team_model_aliases/*")] path: PathBuf,
) {
    let bytes = fs::read(path).unwrap();
    if let Ok(text) = str::from_utf8(&bytes) {
        let stored = decode_model_aliases(&ModelAliases::Text(text.to_owned())).unwrap();
        assert_eq!(stored, serde_json::from_str::<ModelAliasMap>(text).ok());
    }
    if let Ok(ModelAliases::Map(column)) = serde_json::from_slice::<ModelAliases>(&bytes) {
        let checked = decode_model_aliases(&ModelAliases::Map(column))
            .ok()
            .flatten();
        assert_eq!(
            checked,
            serde_json::from_slice::<ModelAliasMap>(&bytes).ok()
        );
    }
}
