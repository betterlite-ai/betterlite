from typing import Final, NotRequired, TypedDict

import litellm
from litellm.assistants.utils import get_optional_params_add_message
from pydantic import JsonValue as Json, TypeAdapter

JSON_MAP: Final[TypeAdapter[dict[str, Json]]] = TypeAdapter(dict[str, Json])


class Input(TypedDict):
    custom_llm_provider: str
    drop_params: bool
    role: NotRequired[Json]
    content: NotRequired[Json]
    attachments: NotRequired[Json]
    metadata: NotRequired[Json]
    kwargs: NotRequired[dict[str, Json]]


def run(input_value: Input) -> dict[str, Json]:
    litellm.drop_params = input_value["drop_params"]
    arguments: dict[str, Json] = {
        "custom_llm_provider": input_value["custom_llm_provider"]
    }
    role = input_value.get("role")
    if role is not None:
        arguments["role"] = role
    content = input_value.get("content")
    if content is not None:
        arguments["content"] = content
    attachments = input_value.get("attachments")
    if attachments is not None:
        arguments["attachments"] = attachments
    metadata = input_value.get("metadata")
    if metadata is not None:
        arguments["metadata"] = metadata
    extras = input_value.get("kwargs")
    if extras is not None:
        arguments.update(extras)
    selected = get_optional_params_add_message(**arguments)
    return JSON_MAP.validate_python(selected)
