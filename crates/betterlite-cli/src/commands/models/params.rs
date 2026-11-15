use indexmap::IndexMap;

use super::ModelsError;

pub(super) fn parse_pairs(
    values: &[String],
    flag: &'static str,
) -> Result<IndexMap<String, String>, ModelsError> {
    let mut pairs = IndexMap::new();
    for value in values {
        let Some((key, item)) = value.split_once('=') else {
            return Err(ModelsError::Pair {
                flag,
                value: value.clone(),
            });
        };
        if pairs.insert(key.to_owned(), item.to_owned()).is_some() {
            return Err(ModelsError::Duplicate {
                flag,
                key: key.to_owned(),
            });
        }
    }
    Ok(pairs)
}
