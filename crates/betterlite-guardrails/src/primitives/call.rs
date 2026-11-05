use indexmap::IndexMap;
use serde::Serialize;

pub mod arguments;

// One function per primitive group: each arm binds the call's arguments to one struct and computes the result from it.
macro_rules! dispatch {
    ($name:ident($kind:ty $(, $state:ident: $state_type:ty)?) -> $output_type:ty {
        $($variant:path, $arguments:ty, |$bound:ident| $output:expr;)+
    }) => {
        pub(super) fn $name(
            primitive: Primitive,
            kind: $kind,
            positional: Positional,
            named: Named
            $(, $state: $state_type)?
        ) -> Result<$output_type, PrimitiveError> {
            Ok(match kind {
                $($variant => {
                    let $bound: $arguments = bind(primitive, positional, named)?;
                    $output
                })+
            })
        }
    };
}

mod dispatch;

use super::code_detection::CodeDetector;
use super::error::PrimitiveError;
use super::http::transport::OutboundHttp;
use super::http::{HttpResponseResult, http_request};
use super::json_value::JsonValue;
use super::registry::Primitive;
use super::results::ActionResult;
use super::urls::UrlExtractor;

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
            Primitive::Action(kind) => dispatch::action(primitive, kind, positional, named),
            Primitive::Regex(kind) => dispatch::regex(primitive, kind, positional, named),
            Primitive::Json(kind) => dispatch::json(primitive, kind, positional, named),
            Primitive::Url(kind) => dispatch::url(primitive, kind, positional, named, &self.urls),
            Primitive::Http(kind) => {
                let request = dispatch::http(primitive, kind, positional, named)?;
                Ok(PrimitiveOutput::Http(
                    http_request(&self.transport, request).await,
                ))
            }
            Primitive::Code(kind) => dispatch::code(primitive, kind, positional, named, &self.code),
            Primitive::Substring(kind) => dispatch::substring(primitive, kind, positional, named),
            Primitive::Text(kind) => dispatch::text(primitive, kind, positional, named),
        }
    }
}
