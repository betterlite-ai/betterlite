from typing import TypedDict

from pydantic import JsonValue, TypeAdapter

from .boundary import CHAT


class ChatCase(TypedDict):
    model: str
    messages: list[JsonValue]
    non_default: dict[str, JsonValue]
    optional_params: dict[str, JsonValue]


def chat_value(case: ChatCase) -> JsonValue:
    optional = dict(case["optional_params"])
    mapped = CHAT.map_openai_params(
        dict(case["non_default"]),
        optional,
        case["model"],
        drop_params=False,
    )
    request = CHAT.transform_request(
        case["model"],
        case["messages"],
        mapped,
        {},
        {},
    )
    return {"mapped": mapped, "request": request}


def chat_cases(params: JsonValue) -> JsonValue:
    cases = TypeAdapter(list[ChatCase]).validate_python(params)
    return {"results": [chat_value(case) for case in cases]}
