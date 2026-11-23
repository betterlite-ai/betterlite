from typing import Final, Literal, TypeAlias, TypedDict

from litellm.router_strategy.complexity_router.fuse_presets import (
    ProfileText,
    get_fuse_presets,
    resolve_fuse_profile,
)
from pydantic import JsonValue, TypeAdapter, ValidationError

CATALOG: Final = TypeAdapter(dict[str, JsonValue])
PROFILE_TEXT: Final = TypeAdapter(ProfileText)


class CatalogCall(TypedDict):
    function: Literal["catalog"]


class ResolveCall(TypedDict):
    function: Literal["resolve"]
    text: str | None
    preset_id: str | None
    kind: Literal["model", "harness"]


class ProfileTextCall(TypedDict):
    function: Literal["profile_text"]
    text: str


Input: TypeAlias = CatalogCall | ResolveCall | ProfileTextCall


class TextAnswer(TypedDict):
    kind: Literal["accepted", "too_short", "too_long"]
    text: str | None


Answer: TypeAlias = TextAnswer | dict[str, JsonValue] | str | None


def rejected(error: ValidationError) -> TextAnswer:
    failure = error.errors()[0]["type"]
    if failure == "string_too_short":
        return {"kind": "too_short", "text": None}
    if failure == "string_too_long":
        return {"kind": "too_long", "text": None}
    raise ValueError(f"unexpected profile text failure {failure}")


def profile_text_answer(call: ProfileTextCall) -> TextAnswer:
    try:
        text = PROFILE_TEXT.validate_python(call["text"])
    except ValidationError as error:
        return rejected(error)
    return {"kind": "accepted", "text": text}


def run(input_value: Input) -> Answer:
    match input_value["function"]:
        case "catalog":
            return CATALOG.validate_json(get_fuse_presets().model_dump_json())
        case "resolve":
            return resolve_fuse_profile(
                input_value["text"], input_value["preset_id"], input_value["kind"]
            )
        case "profile_text":
            return profile_text_answer(input_value)
