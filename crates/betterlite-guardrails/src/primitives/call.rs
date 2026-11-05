use indexmap::IndexMap;
use serde::Serialize;

pub mod arguments;
mod stateless;

use arguments::{
    HttpGetArguments, HttpPostArguments, HttpRequestArguments, LanguagesArguments, TextArguments,
    UrlArguments, bind,
};

use super::code_detection::CodeDetector;
use super::error::PrimitiveError;
use super::http::transport::OutboundHttp;
use super::http::{HttpRequest, HttpResponseResult, http_request};
use super::json_value::JsonValue;
use super::registry::{CodePrimitive, HttpPrimitive, Primitive, UrlPrimitive};
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
            Primitive::Action(kind) => stateless::action(primitive, kind, positional, named),
            Primitive::Regex(kind) => stateless::regex(primitive, kind, positional, named),
            Primitive::Json(kind) => stateless::json(primitive, kind, positional, named),
            Primitive::Url(kind) => self.url(primitive, kind, positional, named),
            Primitive::Http(kind) => self.http(primitive, kind, positional, named).await,
            Primitive::Code(kind) => self.code(primitive, kind, positional, named),
            Primitive::Substring(kind) => stateless::substring(primitive, kind, positional, named),
            Primitive::Text(kind) => stateless::text(primitive, kind, positional, named),
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
