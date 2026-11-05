use indexmap::IndexMap;
use serde::Serialize;

pub mod arguments;

use arguments::{
    BlockArguments, FlagArguments, HttpGetArguments, HttpPostArguments, HttpRequestArguments,
    LanguagesArguments, ModifyArguments, NoArguments, PatternArguments, ReplaceArguments,
    SchemaArguments, StringifyArguments, SubstringArguments, SubstringsArguments, TextArguments,
    UrlArguments, bind,
};

use super::code_detection::CodeDetector;
use super::error::PrimitiveError;
use super::http::transport::OutboundHttp;
use super::http::{HttpRequest, HttpResponseResult, http_request};
use super::json_tools::{json_parse, json_schema_valid, json_stringify};
use super::json_value::JsonValue;
use super::pattern_matching::{regex_find_all, regex_match, regex_match_all, regex_replace};
use super::registry::{
    ActionPrimitive, CodePrimitive, HttpPrimitive, JsonPrimitive, Primitive, RegexPrimitive,
    SubstringPrimitive, TextPrimitive, UrlPrimitive,
};
use super::results::ActionResult;
use super::urls::{UrlExtractor, get_url_domain, is_valid_url};

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum PrimitiveOutput {
    Action(ActionResult),
    Truth(bool),
    Count(usize),
    Text(String),
    Domain(Option<String>),
    Strings(Vec<String>),
    Value(JsonValue),
    Http(HttpResponseResult),
}

type Positional = Vec<JsonValue>;
type Named = IndexMap<String, JsonValue>;

pub struct Primitives<Transport> {
    transport: Transport,
    code: CodeDetector,
    urls: UrlExtractor,
}

impl<Transport: OutboundHttp> Primitives<Transport> {
    pub fn new(transport: Transport) -> Result<Self, PrimitiveError> {
        Ok(Self {
            transport,
            code: CodeDetector::new()?,
            urls: UrlExtractor::new()?,
        })
    }

    pub async fn call(
        &self,
        primitive: Primitive,
        positional: Positional,
        named: Named,
    ) -> Result<PrimitiveOutput, PrimitiveError> {
        match primitive {
            Primitive::Action(kind) => action(primitive, kind, positional, named),
            Primitive::Regex(kind) => regex(primitive, kind, positional, named),
            Primitive::Json(kind) => json(primitive, kind, positional, named),
            Primitive::Url(kind) => self.url(primitive, kind, positional, named),
            Primitive::Http(kind) => self.http(primitive, kind, positional, named).await,
            Primitive::Code(kind) => self.code(primitive, kind, positional, named),
            Primitive::Substring(kind) => substring(primitive, kind, positional, named),
            Primitive::Text(kind) => text(primitive, kind, positional, named),
        }
    }

    fn url(
        &self,
        primitive: Primitive,
        kind: UrlPrimitive,
        positional: Positional,
        named: Named,
    ) -> Result<PrimitiveOutput, PrimitiveError> {
        Ok(match kind {
            UrlPrimitive::Extract => {
                let arguments: TextArguments = bind(primitive, positional, named)?;
                PrimitiveOutput::Strings(self.urls.extract_urls(&arguments.text))
            }
            UrlPrimitive::IsValid => {
                let arguments: UrlArguments = bind(primitive, positional, named)?;
                PrimitiveOutput::Truth(is_valid_url(&arguments.url))
            }
            UrlPrimitive::AllValid => {
                let arguments: TextArguments = bind(primitive, positional, named)?;
                PrimitiveOutput::Truth(self.urls.all_urls_valid(&arguments.text))
            }
            UrlPrimitive::GetDomain => {
                let arguments: UrlArguments = bind(primitive, positional, named)?;
                PrimitiveOutput::Domain(get_url_domain(&arguments.url))
            }
        })
    }

    async fn http(
        &self,
        primitive: Primitive,
        kind: HttpPrimitive,
        positional: Positional,
        named: Named,
    ) -> Result<PrimitiveOutput, PrimitiveError> {
        let request = match kind {
            HttpPrimitive::Request => {
                let arguments: HttpRequestArguments = bind(primitive, positional, named)?;
                HttpRequest {
                    url: arguments.url,
                    method: arguments.method.unwrap_or_else(|| "GET".to_owned()),
                    headers: arguments.headers,
                    body: arguments.body.unwrap_or_default(),
                    timeout: arguments.timeout,
                }
            }
            HttpPrimitive::Get => {
                let arguments: HttpGetArguments = bind(primitive, positional, named)?;
                HttpRequest {
                    url: arguments.url,
                    method: "GET".to_owned(),
                    headers: arguments.headers,
                    body: JsonValue::Null,
                    timeout: arguments.timeout,
                }
            }
            HttpPrimitive::Post => {
                let arguments: HttpPostArguments = bind(primitive, positional, named)?;
                HttpRequest {
                    url: arguments.url,
                    method: "POST".to_owned(),
                    headers: arguments.headers,
                    body: arguments.body.unwrap_or_default(),
                    timeout: arguments.timeout,
                }
            }
        };
        Ok(PrimitiveOutput::Http(
            http_request(&self.transport, request).await,
        ))
    }

    fn code(
        &self,
        primitive: Primitive,
        kind: CodePrimitive,
        positional: Positional,
        named: Named,
    ) -> Result<PrimitiveOutput, PrimitiveError> {
        Ok(match kind {
            CodePrimitive::Detect => {
                let arguments: TextArguments = bind(primitive, positional, named)?;
                PrimitiveOutput::Truth(!self.code.detect_code_languages(&arguments.text).is_empty())
            }
            CodePrimitive::DetectLanguages => {
                let arguments: TextArguments = bind(primitive, positional, named)?;
                let detected = self.code.detect_code_languages(&arguments.text);
                PrimitiveOutput::Strings(detected.into_iter().map(str::to_owned).collect())
            }
            CodePrimitive::ContainsLanguage => {
                let arguments: LanguagesArguments = bind(primitive, positional, named)?;
                PrimitiveOutput::Truth(
                    self.code
                        .contains_code_language(&arguments.text, &arguments.languages),
                )
            }
        })
    }
}

fn action(
    primitive: Primitive,
    kind: ActionPrimitive,
    positional: Positional,
    named: Named,
) -> Result<PrimitiveOutput, PrimitiveError> {
    let result = match kind {
        ActionPrimitive::Allow => {
            let _none: NoArguments = bind(primitive, positional, named)?;
            ActionResult::Allow
        }
        ActionPrimitive::Block => {
            let arguments: BlockArguments = bind(primitive, positional, named)?;
            ActionResult::Block {
                reason: arguments.reason,
                detection_info: arguments.detection_info.filter(|info| !info.is_empty()),
            }
        }
        ActionPrimitive::Flag => {
            let arguments: FlagArguments = bind(primitive, positional, named)?;
            ActionResult::Flag {
                reason: arguments.reason,
                metadata: arguments.metadata.unwrap_or_default(),
            }
        }
        ActionPrimitive::Modify => {
            let arguments: ModifyArguments = bind(primitive, positional, named)?;
            ActionResult::Modify {
                texts: arguments.texts,
                images: arguments.images,
                tool_calls: arguments.tool_calls,
            }
        }
    };
    Ok(PrimitiveOutput::Action(result))
}

fn regex(
    primitive: Primitive,
    kind: RegexPrimitive,
    positional: Positional,
    named: Named,
) -> Result<PrimitiveOutput, PrimitiveError> {
    Ok(match kind {
        RegexPrimitive::Match => {
            let arguments: PatternArguments = bind(primitive, positional, named)?;
            PrimitiveOutput::Truth(regex_match(
                &arguments.text,
                &arguments.pattern,
                arguments.flags.unwrap_or_default(),
            ))
        }
        RegexPrimitive::MatchAll => {
            let arguments: PatternArguments = bind(primitive, positional, named)?;
            PrimitiveOutput::Truth(regex_match_all(
                &arguments.text,
                &arguments.pattern,
                arguments.flags.unwrap_or_default(),
            ))
        }
        RegexPrimitive::Replace => {
            let arguments: ReplaceArguments = bind(primitive, positional, named)?;
            let flags = arguments.flags.unwrap_or_default();
            PrimitiveOutput::Text(regex_replace(
                &arguments.text,
                &arguments.pattern,
                &arguments.replacement,
                flags,
            ))
        }
        RegexPrimitive::FindAll => {
            let arguments: PatternArguments = bind(primitive, positional, named)?;
            let found = regex_find_all(
                &arguments.text,
                &arguments.pattern,
                arguments.flags.unwrap_or_default(),
            );
            PrimitiveOutput::Value(JsonValue::Array(found))
        }
    })
}

fn json(
    primitive: Primitive,
    kind: JsonPrimitive,
    positional: Positional,
    named: Named,
) -> Result<PrimitiveOutput, PrimitiveError> {
    Ok(match kind {
        JsonPrimitive::Parse => {
            let arguments: TextArguments = bind(primitive, positional, named)?;
            PrimitiveOutput::Value(json_parse(&arguments.text))
        }
        JsonPrimitive::Stringify => {
            let arguments: StringifyArguments = bind(primitive, positional, named)?;
            PrimitiveOutput::Text(json_stringify(&arguments.obj))
        }
        JsonPrimitive::SchemaValid => {
            let arguments: SchemaArguments = bind(primitive, positional, named)?;
            PrimitiveOutput::Truth(json_schema_valid(&arguments.obj, &arguments.schema))
        }
    })
}

fn substring(
    primitive: Primitive,
    kind: SubstringPrimitive,
    positional: Positional,
    named: Named,
) -> Result<PrimitiveOutput, PrimitiveError> {
    Ok(match kind {
        SubstringPrimitive::Contains => {
            let arguments: SubstringArguments = bind(primitive, positional, named)?;
            PrimitiveOutput::Truth(arguments.text.contains(&arguments.substring))
        }
        SubstringPrimitive::ContainsAny => {
            let arguments: SubstringsArguments = bind(primitive, positional, named)?;
            PrimitiveOutput::Truth(
                arguments
                    .substrings
                    .iter()
                    .any(|part| arguments.text.contains(part.as_str())),
            )
        }
        SubstringPrimitive::ContainsAll => {
            let arguments: SubstringsArguments = bind(primitive, positional, named)?;
            PrimitiveOutput::Truth(
                arguments
                    .substrings
                    .iter()
                    .all(|part| arguments.text.contains(part.as_str())),
            )
        }
    })
}

fn text(
    primitive: Primitive,
    kind: TextPrimitive,
    positional: Positional,
    named: Named,
) -> Result<PrimitiveOutput, PrimitiveError> {
    let arguments: TextArguments = bind(primitive, positional, named)?;
    Ok(match kind {
        TextPrimitive::WordCount => {
            PrimitiveOutput::Count(arguments.text.split_whitespace().count())
        }
        TextPrimitive::CharCount => PrimitiveOutput::Count(arguments.text.chars().count()),
        TextPrimitive::Lower => PrimitiveOutput::Text(arguments.text.to_lowercase()),
        TextPrimitive::Upper => PrimitiveOutput::Text(arguments.text.to_uppercase()),
        TextPrimitive::Trim => PrimitiveOutput::Text(arguments.text.trim().to_owned()),
    })
}
