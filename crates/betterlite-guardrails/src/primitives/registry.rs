use indexmap::IndexMap;

// name and parameter names in positional order
pub type Signature = (&'static str, &'static [&'static str]);

const TEXT: &[&str] = &["text"];
const PATTERN: &[&str] = &["text", "pattern", "flags"];
const URL: &[&str] = &["url"];
const SUBSTRINGS: &[&str] = &["text", "substrings"];
const REPLACE: &[&str] = &["text", "pattern", "replacement", "flags"];
const REQUEST: &[&str] = &["url", "method", "headers", "body", "timeout"];
const POST: &[&str] = &["url", "body", "headers", "timeout"];
const LANGUAGES: &[&str] = &["text", "languages"];

// The safe builtins custom code may call; the Starlark runtime provides them.
const BUILTINS: [&str; 10] = [
    "len", "str", "int", "float", "bool", "list", "dict", "True", "False", "None",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Primitive {
    Action(ActionPrimitive),
    Regex(RegexPrimitive),
    Json(JsonPrimitive),
    Url(UrlPrimitive),
    Http(HttpPrimitive),
    Code(CodePrimitive),
    Substring(SubstringPrimitive),
    Text(TextPrimitive),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionPrimitive {
    Allow,
    Block,
    Flag,
    Modify,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegexPrimitive {
    Match,
    MatchAll,
    Replace,
    FindAll,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsonPrimitive {
    Parse,
    Stringify,
    SchemaValid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UrlPrimitive {
    Extract,
    IsValid,
    AllValid,
    GetDomain,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpPrimitive {
    Request,
    Get,
    Post,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodePrimitive {
    Detect,
    DetectLanguages,
    ContainsLanguage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubstringPrimitive {
    Contains,
    ContainsAny,
    ContainsAll,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextPrimitive {
    WordCount,
    CharCount,
    Lower,
    Upper,
    Trim,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistryEntry {
    Function(Primitive),
    Builtin(&'static str),
}

impl Primitive {
    pub const ALL: [Self; 29] = [
        Self::Action(ActionPrimitive::Allow),
        Self::Action(ActionPrimitive::Block),
        Self::Action(ActionPrimitive::Flag),
        Self::Action(ActionPrimitive::Modify),
        Self::Regex(RegexPrimitive::Match),
        Self::Regex(RegexPrimitive::MatchAll),
        Self::Regex(RegexPrimitive::Replace),
        Self::Regex(RegexPrimitive::FindAll),
        Self::Json(JsonPrimitive::Parse),
        Self::Json(JsonPrimitive::Stringify),
        Self::Json(JsonPrimitive::SchemaValid),
        Self::Url(UrlPrimitive::Extract),
        Self::Url(UrlPrimitive::IsValid),
        Self::Url(UrlPrimitive::AllValid),
        Self::Url(UrlPrimitive::GetDomain),
        Self::Http(HttpPrimitive::Request),
        Self::Http(HttpPrimitive::Get),
        Self::Http(HttpPrimitive::Post),
        Self::Code(CodePrimitive::Detect),
        Self::Code(CodePrimitive::DetectLanguages),
        Self::Code(CodePrimitive::ContainsLanguage),
        Self::Substring(SubstringPrimitive::Contains),
        Self::Substring(SubstringPrimitive::ContainsAny),
        Self::Substring(SubstringPrimitive::ContainsAll),
        Self::Text(TextPrimitive::WordCount),
        Self::Text(TextPrimitive::CharCount),
        Self::Text(TextPrimitive::Lower),
        Self::Text(TextPrimitive::Upper),
        Self::Text(TextPrimitive::Trim),
    ];

    #[must_use]
    pub const fn signature(self) -> Signature {
        match self {
            Self::Action(ActionPrimitive::Allow) => ("allow", &[]),
            Self::Action(ActionPrimitive::Block) => ("block", &["reason", "detection_info"]),
            Self::Action(ActionPrimitive::Flag) => ("flag", &["reason", "metadata"]),
            Self::Action(ActionPrimitive::Modify) => ("modify", &["texts", "images", "tool_calls"]),
            Self::Regex(RegexPrimitive::Match) => ("regex_match", PATTERN),
            Self::Regex(RegexPrimitive::MatchAll) => ("regex_match_all", PATTERN),
            Self::Regex(RegexPrimitive::Replace) => ("regex_replace", REPLACE),
            Self::Regex(RegexPrimitive::FindAll) => ("regex_find_all", PATTERN),
            Self::Json(JsonPrimitive::Parse) => ("json_parse", TEXT),
            Self::Json(JsonPrimitive::Stringify) => ("json_stringify", &["obj"]),
            Self::Json(JsonPrimitive::SchemaValid) => ("json_schema_valid", &["obj", "schema"]),
            Self::Url(UrlPrimitive::Extract) => ("extract_urls", TEXT),
            Self::Url(UrlPrimitive::IsValid) => ("is_valid_url", URL),
            Self::Url(UrlPrimitive::AllValid) => ("all_urls_valid", TEXT),
            Self::Url(UrlPrimitive::GetDomain) => ("get_url_domain", URL),
            Self::Http(HttpPrimitive::Request) => ("http_request", REQUEST),
            Self::Http(HttpPrimitive::Get) => ("http_get", &["url", "headers", "timeout"]),
            Self::Http(HttpPrimitive::Post) => ("http_post", POST),
            Self::Code(CodePrimitive::Detect) => ("detect_code", TEXT),
            Self::Code(CodePrimitive::DetectLanguages) => ("detect_code_languages", TEXT),
            Self::Code(CodePrimitive::ContainsLanguage) => ("contains_code_language", LANGUAGES),
            Self::Substring(SubstringPrimitive::Contains) => ("contains", &["text", "substring"]),
            Self::Substring(SubstringPrimitive::ContainsAny) => ("contains_any", SUBSTRINGS),
            Self::Substring(SubstringPrimitive::ContainsAll) => ("contains_all", SUBSTRINGS),
            Self::Text(TextPrimitive::WordCount) => ("word_count", TEXT),
            Self::Text(TextPrimitive::CharCount) => ("char_count", TEXT),
            Self::Text(TextPrimitive::Lower) => ("lower", TEXT),
            Self::Text(TextPrimitive::Upper) => ("upper", TEXT),
            Self::Text(TextPrimitive::Trim) => ("trim", TEXT),
        }
    }
}

#[must_use]
pub fn get_custom_code_primitives() -> IndexMap<&'static str, RegistryEntry> {
    let functions = Primitive::ALL
        .into_iter()
        .map(|primitive| (primitive.signature().0, RegistryEntry::Function(primitive)));
    let builtins = BUILTINS
        .into_iter()
        .map(|name| (name, RegistryEntry::Builtin(name)));
    functions.chain(builtins).collect()
}
