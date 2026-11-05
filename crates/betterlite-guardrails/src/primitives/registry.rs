use indexmap::IndexMap;

// name and parameter names in positional order
pub type Signature = (&'static str, &'static [&'static str]);

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
pub enum Builtin {
    Len,
    Str,
    Int,
    Float,
    Bool,
    List,
    Dict,
    True,
    False,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistryEntry {
    Function(Primitive),
    Builtin(Builtin),
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
            Self::Action(kind) => kind.signature(),
            Self::Regex(kind) => kind.signature(),
            Self::Json(kind) => kind.signature(),
            Self::Url(kind) => kind.signature(),
            Self::Http(kind) => kind.signature(),
            Self::Code(kind) => kind.signature(),
            Self::Substring(kind) => kind.signature(),
            Self::Text(kind) => kind.signature(),
        }
    }
}

impl ActionPrimitive {
    const fn signature(self) -> Signature {
        match self {
            Self::Allow => ("allow", &[]),
            Self::Block => ("block", &["reason", "detection_info"]),
            Self::Flag => ("flag", &["reason", "metadata"]),
            Self::Modify => ("modify", &["texts", "images", "tool_calls"]),
        }
    }
}

impl RegexPrimitive {
    const fn signature(self) -> Signature {
        match self {
            Self::Match => ("regex_match", &["text", "pattern", "flags"]),
            Self::MatchAll => ("regex_match_all", &["text", "pattern", "flags"]),
            Self::Replace => (
                "regex_replace",
                &["text", "pattern", "replacement", "flags"],
            ),
            Self::FindAll => ("regex_find_all", &["text", "pattern", "flags"]),
        }
    }
}

impl JsonPrimitive {
    const fn signature(self) -> Signature {
        match self {
            Self::Parse => ("json_parse", &["text"]),
            Self::Stringify => ("json_stringify", &["obj"]),
            Self::SchemaValid => ("json_schema_valid", &["obj", "schema"]),
        }
    }
}

impl UrlPrimitive {
    const fn signature(self) -> Signature {
        match self {
            Self::Extract => ("extract_urls", &["text"]),
            Self::IsValid => ("is_valid_url", &["url"]),
            Self::AllValid => ("all_urls_valid", &["text"]),
            Self::GetDomain => ("get_url_domain", &["url"]),
        }
    }
}

impl HttpPrimitive {
    const fn signature(self) -> Signature {
        match self {
            Self::Request => (
                "http_request",
                &["url", "method", "headers", "body", "timeout"],
            ),
            Self::Get => ("http_get", &["url", "headers", "timeout"]),
            Self::Post => ("http_post", &["url", "body", "headers", "timeout"]),
        }
    }
}

impl CodePrimitive {
    const fn signature(self) -> Signature {
        match self {
            Self::Detect => ("detect_code", &["text"]),
            Self::DetectLanguages => ("detect_code_languages", &["text"]),
            Self::ContainsLanguage => ("contains_code_language", &["text", "languages"]),
        }
    }
}

impl SubstringPrimitive {
    const fn signature(self) -> Signature {
        match self {
            Self::Contains => ("contains", &["text", "substring"]),
            Self::ContainsAny => ("contains_any", &["text", "substrings"]),
            Self::ContainsAll => ("contains_all", &["text", "substrings"]),
        }
    }
}

impl TextPrimitive {
    const fn signature(self) -> Signature {
        match self {
            Self::WordCount => ("word_count", &["text"]),
            Self::CharCount => ("char_count", &["text"]),
            Self::Lower => ("lower", &["text"]),
            Self::Upper => ("upper", &["text"]),
            Self::Trim => ("trim", &["text"]),
        }
    }
}

impl Builtin {
    pub const ALL: [Self; 10] = [
        Self::Len,
        Self::Str,
        Self::Int,
        Self::Float,
        Self::Bool,
        Self::List,
        Self::Dict,
        Self::True,
        Self::False,
        Self::None,
    ];

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Len => "len",
            Self::Str => "str",
            Self::Int => "int",
            Self::Float => "float",
            Self::Bool => "bool",
            Self::List => "list",
            Self::Dict => "dict",
            Self::True => "True",
            Self::False => "False",
            Self::None => "None",
        }
    }
}

#[must_use]
pub fn get_custom_code_primitives() -> IndexMap<&'static str, RegistryEntry> {
    let functions = Primitive::ALL
        .into_iter()
        .map(|primitive| (primitive.signature().0, RegistryEntry::Function(primitive)));
    let builtins = Builtin::ALL
        .into_iter()
        .map(|builtin| (builtin.name(), RegistryEntry::Builtin(builtin)));
    functions.chain(builtins).collect()
}
