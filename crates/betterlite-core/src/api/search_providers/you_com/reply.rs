use betterlite_openai::openai_base_config::Object;
use betterlite_openai::search::records::{SearchObject, SearchResponse, SearchResult};
use serde::Deserialize;
use serde_json::Deserializer;

use super::error::YouComError;

pub const MAX_REPLY_BYTES: usize = 0x0100_0000;

#[derive(Deserialize, Default)]
struct YouComReply {
    #[serde(default)]
    results: Option<ResultsField>,
    // a flattened map keeps serde from accepting a JSON array in place of this object
    #[serde(flatten)]
    _extra: Object,
}

// LiteLLM's `results or {}` turns an empty list into the default object while a nonempty list fails
#[derive(Deserialize)]
#[serde(untagged)]
enum ResultsField {
    Empty([(); 0]),
    Results(YouComResults),
}

#[derive(Deserialize, Default)]
struct YouComResults {
    #[serde(default)]
    web: Option<Vec<YouComResult>>,
    #[serde(default)]
    news: Option<Vec<YouComResult>>,
    // a flattened map keeps serde from accepting a JSON array in place of this object
    #[serde(flatten)]
    _extra: Object,
}

type OptionalDescription = Option<Option<String>>;

#[derive(Deserialize)]
struct YouComResult {
    #[serde(default)]
    title: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    snippets: Option<Vec<String>>,
    // LiteLLM reads description only when snippets is empty, where absent means "" and null fails validation
    #[serde(default, with = "serde_with::rust::double_option")]
    description: OptionalDescription,
    #[serde(default)]
    page_age: Option<String>,
    // a flattened map keeps serde from accepting a JSON array in place of this object
    #[serde(flatten)]
    _extra: Object,
}

pub fn decode_you_com_reply(bytes: &[u8]) -> Result<SearchResponse, YouComError> {
    if bytes.len() > MAX_REPLY_BYTES {
        return Err(YouComError::ReplyTooLarge {
            limit: MAX_REPLY_BYTES,
        });
    }
    let mut decoder = Deserializer::from_slice(bytes);
    let reply: YouComReply = serde_path_to_error::deserialize(&mut decoder)
        .map_err(|source| YouComError::Reply { source })?;
    decoder.end().map_err(YouComError::TrailingBytes)?;
    let results = match reply.results {
        None | Some(ResultsField::Empty([])) => Vec::new(),
        Some(ResultsField::Results(container)) => container.into_results()?,
    };
    Ok(SearchResponse {
        results,
        object: SearchObject::default(),
        extra: Object::new(),
        hidden_params: Object::new(),
    })
}

impl YouComResults {
    fn into_results(self) -> Result<Vec<SearchResult>, YouComError> {
        let Self { web, news, .. } = self;
        web.into_iter()
            .flatten()
            .chain(news.into_iter().flatten())
            .map(project)
            .collect()
    }
}

fn project(item: YouComResult) -> Result<SearchResult, YouComError> {
    let YouComResult {
        title,
        url,
        snippets,
        description,
        page_age,
        ..
    } = item;
    let snippet = match snippets.and_then(|list| list.into_iter().next()) {
        Some(snippet) => snippet,
        None => match description {
            None => String::new(),
            Some(None) => return Err(YouComError::DescriptionNotString),
            Some(Some(text)) => text,
        },
    };
    Ok(SearchResult {
        title,
        url,
        snippet,
        date: page_age,
        last_updated: None,
        extra: Object::new(),
    })
}
