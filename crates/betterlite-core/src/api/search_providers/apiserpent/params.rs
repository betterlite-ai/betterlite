use betterlite_openai::openai_base_config::Object;
use indexmap::IndexMap;

use super::super::google_pse::wire::SearchParam;
use super::ApiserpentSearchError;

pub(super) const NUM_MIN: i64 = 1;
pub(super) const NUM_MIN_DEEP: i64 = 10;
pub(super) const NUM_MAX: i64 = 100;
const PAGES_MIN: i64 = 1;
const PAGES_MAX: i64 = 10;

const FIELD_NAMES: [&str; 9] = [
    "engine",
    "country",
    "num",
    "format",
    "pages",
    "freshness",
    "safe",
    "language",
    "pixel_position",
];

#[derive(Debug, Clone)]
pub(super) struct ApiserpentParams {
    engine: String,
    country: String,
    num: i64,
    format: String,
    pages: Option<i64>,
    freshness: Option<String>,
    safe: Option<String>,
    language: Option<String>,
    pixel_position: Option<bool>,
}

impl ApiserpentParams {
    pub(super) fn from_optional(
        optional_params: &Object,
        deep: bool,
    ) -> Result<Self, ApiserpentSearchError> {
        let mut params = Self {
            engine: String::from("google"),
            country: String::from("us"),
            num: 10,
            format: String::from("full"),
            pages: None,
            freshness: None,
            safe: None,
            language: None,
            pixel_position: None,
        };
        params.apply_max_results(optional_params, deep)?;
        params.apply_recognized(optional_params)?;
        params.validate()?;
        Ok(params)
    }

    fn apply_max_results(
        &mut self,
        optional_params: &Object,
        deep: bool,
    ) -> Result<(), ApiserpentSearchError> {
        if !max_results_present(optional_params) {
            return Ok(());
        }
        let ceiling = number(optional_params, "max_results")?;
        let floor = if deep { NUM_MIN_DEEP } else { NUM_MIN };
        self.num = ceiling.clamp(floor, NUM_MAX);
        Ok(())
    }

    fn apply_recognized(&mut self, optional_params: &Object) -> Result<(), ApiserpentSearchError> {
        for name in FIELD_NAMES {
            if name == "num" && max_results_present(optional_params) {
                continue;
            }
            let Some(value) = optional_params.get(name) else {
                continue;
            };
            if value.is_null() {
                continue;
            }
            match name {
                "engine" => self.engine = text(optional_params, name)?,
                "country" => self.country = text(optional_params, name)?.to_lowercase(),
                "num" => self.num = number(optional_params, name)?,
                "format" => self.format = text(optional_params, name)?,
                "pages" => self.pages = Some(number(optional_params, name)?),
                "freshness" => self.freshness = Some(text(optional_params, name)?),
                "safe" => self.safe = Some(text(optional_params, name)?),
                "language" => self.language = Some(text(optional_params, name)?),
                "pixel_position" => {
                    self.pixel_position = Some(boolean(optional_params, name)?);
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn validate(&self) -> Result<(), ApiserpentSearchError> {
        if !(NUM_MIN..=NUM_MAX).contains(&self.num) {
            return Err(ApiserpentSearchError::NumRange { value: self.num });
        }
        if let Some(pages) = self.pages
            && !(PAGES_MIN..=PAGES_MAX).contains(&pages)
        {
            return Err(ApiserpentSearchError::PagesRange { value: pages });
        }
        Ok(())
    }

    pub(super) fn to_request_params(&self, query: String) -> IndexMap<String, SearchParam> {
        let mut params = IndexMap::new();
        params.insert("engine".to_owned(), SearchParam::Text(self.engine.clone()));
        params.insert(
            "country".to_owned(),
            SearchParam::Text(self.country.clone()),
        );
        params.insert("num".to_owned(), SearchParam::Number(self.num));
        params.insert("format".to_owned(), SearchParam::Text(self.format.clone()));
        if let Some(pages) = self.pages {
            params.insert("pages".to_owned(), SearchParam::Number(pages));
        }
        if let Some(freshness) = &self.freshness {
            params.insert("freshness".to_owned(), SearchParam::Text(freshness.clone()));
        }
        if let Some(safe) = &self.safe {
            params.insert("safe".to_owned(), SearchParam::Text(safe.clone()));
        }
        if let Some(language) = &self.language {
            params.insert("language".to_owned(), SearchParam::Text(language.clone()));
        }
        if let Some(pixel_position) = self.pixel_position {
            let flag = if pixel_position { "true" } else { "false" };
            params.insert(
                "pixel_position".to_owned(),
                SearchParam::Text(flag.to_owned()),
            );
        }
        params.insert("q".to_owned(), SearchParam::Text(query));
        params
    }
}

fn max_results_present(optional_params: &Object) -> bool {
    optional_params
        .get("max_results")
        .is_some_and(|value| !value.is_null())
}

fn text(optional_params: &Object, name: &'static str) -> Result<String, ApiserpentSearchError> {
    optional_params
        .get(name)
        .and_then(|value| value.as_str())
        .map(str::to_owned)
        .ok_or(ApiserpentSearchError::OptionType { field: name })
}

fn number(optional_params: &Object, name: &'static str) -> Result<i64, ApiserpentSearchError> {
    let value = optional_params
        .get(name)
        .ok_or(ApiserpentSearchError::OptionType { field: name })?;
    value
        .as_i64()
        .ok_or(ApiserpentSearchError::OptionType { field: name })
}

fn boolean(optional_params: &Object, name: &'static str) -> Result<bool, ApiserpentSearchError> {
    let value = optional_params
        .get(name)
        .ok_or(ApiserpentSearchError::OptionType { field: name })?;
    value
        .as_bool()
        .ok_or(ApiserpentSearchError::OptionType { field: name })
}
