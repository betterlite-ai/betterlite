use super::super::code_detection::CodeDetector;
use super::super::error::PrimitiveError;
use super::super::http::HttpRequest;
use super::super::json_tools::{json_parse, json_schema_valid, json_stringify};
use super::super::json_value::JsonValue;
use super::super::pattern_matching::{compile, find_all, or_warn, regex_match_all, replace_all};
use super::super::registry::{
    ActionPrimitive, CodePrimitive, HttpPrimitive, JsonPrimitive, Primitive, RegexPrimitive,
    SubstringPrimitive, TextPrimitive, UrlPrimitive,
};
use super::super::results::ActionResult;
use super::super::urls::{UrlExtractor, get_url_domain, is_valid_url};
use super::arguments::{
    BlockArguments, FlagArguments, HttpGetArguments, HttpPostArguments, HttpRequestArguments,
    LanguagesArguments, ModifyArguments, NoArguments, PatternArguments, ReplaceArguments,
    SchemaArguments, StringifyArguments, SubstringArguments, SubstringsArguments, TextArguments,
    UrlArguments, bind,
};
use super::{Named, Positional, PrimitiveOutput};

dispatch!(action(ActionPrimitive) -> PrimitiveOutput {
    ActionPrimitive::Allow, NoArguments, |_none| PrimitiveOutput::Action(ActionResult::Allow);
    ActionPrimitive::Block, BlockArguments, |args| PrimitiveOutput::Action(ActionResult::Block {
        reason: args.reason,
        detection_info: args.detection_info.filter(|info| !info.is_empty()),
    });
    ActionPrimitive::Flag, FlagArguments, |args| PrimitiveOutput::Action(ActionResult::Flag {
        reason: args.reason,
        metadata: args.metadata,
    });
    ActionPrimitive::Modify, ModifyArguments, |args| PrimitiveOutput::Action(ActionResult::Modify {
        texts: args.texts,
        images: args.images,
        tool_calls: args.tool_calls,
    });
});

dispatch!(regex(RegexPrimitive) -> PrimitiveOutput {
    RegexPrimitive::Match, PatternArguments, |args| PrimitiveOutput::Truth(or_warn(
        compile(&args.pattern, args.flags).and_then(|regex| regex.is_match(&args.text)),
        false,
        "regex_match",
    ));
    RegexPrimitive::MatchAll, PatternArguments, |args| PrimitiveOutput::Truth(
        regex_match_all(&args.text, &args.pattern, args.flags)
    );
    RegexPrimitive::Replace, ReplaceArguments, |args| PrimitiveOutput::Text(or_warn(
        replace_all(&args.text, &args.pattern, &args.replacement, args.flags),
        args.text.clone(),
        "regex_replace",
    ));
    RegexPrimitive::FindAll, PatternArguments, |args| PrimitiveOutput::Value(JsonValue::Array(or_warn(
        find_all(&args.text, &args.pattern, args.flags),
        Vec::new(),
        "regex_find_all",
    )));
});

dispatch!(json(JsonPrimitive) -> PrimitiveOutput {
    JsonPrimitive::Parse, TextArguments, |args| PrimitiveOutput::Value(json_parse(&args.text));
    JsonPrimitive::Stringify, StringifyArguments, |args| PrimitiveOutput::Text(json_stringify(&args.obj));
    JsonPrimitive::SchemaValid, SchemaArguments, |args| PrimitiveOutput::Truth(
        json_schema_valid(&args.obj, &args.schema)
    );
});

dispatch!(url(UrlPrimitive, urls: &UrlExtractor) -> PrimitiveOutput {
    UrlPrimitive::Extract, TextArguments, |args| PrimitiveOutput::Strings(urls.extract_urls(&args.text));
    UrlPrimitive::IsValid, UrlArguments, |args| PrimitiveOutput::Truth(is_valid_url(&args.url));
    UrlPrimitive::AllValid, TextArguments, |args| PrimitiveOutput::Truth(urls.all_urls_valid(&args.text));
    UrlPrimitive::GetDomain, UrlArguments, |args| PrimitiveOutput::Domain(get_url_domain(&args.url));
});

dispatch!(http(HttpPrimitive) -> HttpRequest {
    HttpPrimitive::Request, HttpRequestArguments, |args| HttpRequest {
        url: args.url,
        method: args.method,
        headers: args.headers,
        body: args.body,
        timeout: args.timeout,
    };
    HttpPrimitive::Get, HttpGetArguments, |args| HttpRequest {
        url: args.url,
        method: "GET".to_owned(),
        headers: args.headers,
        body: JsonValue::Null,
        timeout: args.timeout,
    };
    HttpPrimitive::Post, HttpPostArguments, |args| HttpRequest {
        url: args.url,
        method: "POST".to_owned(),
        headers: args.headers,
        body: args.body,
        timeout: args.timeout,
    };
});

dispatch!(code(CodePrimitive, detector: &CodeDetector) -> PrimitiveOutput {
    CodePrimitive::Detect, TextArguments, |args| PrimitiveOutput::Truth(
        !detector.detect_code_languages(&args.text).is_empty()
    );
    CodePrimitive::DetectLanguages, TextArguments, |args| PrimitiveOutput::Strings(
        detector.detect_code_languages(&args.text).into_iter().map(str::to_owned).collect()
    );
    CodePrimitive::ContainsLanguage, LanguagesArguments, |args| PrimitiveOutput::Truth(
        detector.contains_code_language(&args.text, &args.languages)
    );
});

dispatch!(substring(SubstringPrimitive) -> PrimitiveOutput {
    SubstringPrimitive::Contains, SubstringArguments, |args| PrimitiveOutput::Truth(
        args.text.contains(&args.substring)
    );
    SubstringPrimitive::ContainsAny, SubstringsArguments, |args| PrimitiveOutput::Truth(
        args.substrings.iter().any(|part| args.text.contains(part.as_str()))
    );
    SubstringPrimitive::ContainsAll, SubstringsArguments, |args| PrimitiveOutput::Truth(
        args.substrings.iter().all(|part| args.text.contains(part.as_str()))
    );
});

dispatch!(text(TextPrimitive) -> PrimitiveOutput {
    TextPrimitive::WordCount, TextArguments, |args| PrimitiveOutput::Count(args.text.split_whitespace().count());
    TextPrimitive::CharCount, TextArguments, |args| PrimitiveOutput::Count(args.text.chars().count());
    TextPrimitive::Lower, TextArguments, |args| PrimitiveOutput::Text(args.text.to_lowercase());
    TextPrimitive::Upper, TextArguments, |args| PrimitiveOutput::Text(args.text.to_uppercase());
    TextPrimitive::Trim, TextArguments, |args| PrimitiveOutput::Text(args.text.trim().to_owned());
});
