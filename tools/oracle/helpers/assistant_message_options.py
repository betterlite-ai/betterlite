from typing import Final, NotRequired, Protocol, TypedDict, cast

import litellm
from litellm.assistants.utils import get_optional_params_add_message as select_params
from pydantic import JsonValue as Json
from pydantic import TypeAdapter

JSON_MAP: Final[TypeAdapter[dict[str, Json]]] = TypeAdapter(dict[str, Json])


class Input(TypedDict):
    custom_llm_provider: str
    drop_params: bool
    role: NotRequired[Json]
    content: NotRequired[Json]
    attachments: NotRequired[Json]
    metadata: NotRequired[Json]
    kwargs: NotRequired[dict[str, Json]]


class Selector(Protocol):
    def __call__(self, **params: Json) -> dict[str, Json]: ...


SELECT: Selector = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/assistants/utils.py#L10-L66
    Selector,
    select_params,
)


def run(input_value: Input) -> dict[str, Json]:
    litellm.drop_params = input_value["drop_params"]
    extras: dict[str, Json] = input_value["kwargs"] if "kwargs" in input_value else {}
    selected = SELECT(
        role=input_value.get("role"),
        content=input_value.get("content"),
        attachments=input_value.get("attachments"),
        metadata=input_value.get("metadata"),
        custom_llm_provider=input_value["custom_llm_provider"],
        **extras,
    )
    return JSON_MAP.validate_python(selected)
