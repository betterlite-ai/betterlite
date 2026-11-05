use super::super::error::PrimitiveError;
use super::super::json_tools::{json_parse, json_schema_valid, json_stringify};
use super::super::json_value::JsonValue;
use super::super::pattern_matching::{regex_find_all, regex_match, regex_match_all, regex_replace};
use super::super::registry::{
    ActionPrimitive, JsonPrimitive, Primitive, RegexPrimitive, SubstringPrimitive, TextPrimitive,
};
use super::super::results::ActionResult;
use super::arguments::{
    BlockArguments, FlagArguments, ModifyArguments, NoArguments, PatternArguments,
    ReplaceArguments, SchemaArguments, StringifyArguments, SubstringArguments, SubstringsArguments,
    TextArguments, bind,
};
use super::{Named, Positional, PrimitiveOutput};

pub(super) fn action(
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

pub(super) fn regex(
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

pub(super) fn json(
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

pub(super) fn substring(
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

pub(super) fn text(
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
