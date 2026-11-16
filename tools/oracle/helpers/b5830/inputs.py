from typing import Final, Literal, TypeAlias, TypedDict

import litellm
from pydantic import JsonValue, TypeAdapter

# every adapter module imports litellm, as the oracle's helper rule asks
_ = litellm.__name__


Object: TypeAlias = dict[str, JsonValue]
Headers: TypeAlias = dict[str, str]
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)


class ImageBytes(TypedDict):
    kind: Literal["bytes"]
    data: str


class ImageReader(TypedDict):
    kind: Literal["reader"]
    data: str
    position: int


ImageCase: TypeAlias = ImageBytes | ImageReader


class GenerationParamsCase(TypedDict):
    model: str
    non_default_params: Object
    optional_params: Object
    drop_params: bool


class GenerationRequestCase(TypedDict):
    model: str
    prompt: str
    optional_params: Object
    headers: Headers
    api_key: str | None
    global_api_key: str | None
    environment: dict[str, str]
    api_base: str | None


class ReplyCase(TypedDict):
    model: str
    body: str
    status: int
    headers: Headers
    created: int


class EditRequestCase(TypedDict):
    model: str
    prompt: str | None
    image: ImageCase | None
    optional_params: Object
    headers: Headers
    api_key: str | None
    global_api_key: str | None
    environment: dict[str, str]
    api_base: str | None


class GenerationParamsInput(TypedDict):
    op: Literal["generation_params"]
    cases: list[GenerationParamsCase]


class GenerationRequestInput(TypedDict):
    op: Literal["generation_request"]
    cases: list[GenerationRequestCase]


class GenerationReplyInput(TypedDict):
    op: Literal["generation_reply"]
    cases: list[ReplyCase]


class EditParamsInput(TypedDict):
    op: Literal["edit_params"]
    cases: list[Object]


class EditRequestInput(TypedDict):
    op: Literal["edit_request"]
    cases: list[EditRequestCase]


class EditReplyInput(TypedDict):
    op: Literal["edit_reply"]
    cases: list[ReplyCase]


Input: TypeAlias = (
    GenerationParamsInput | GenerationRequestInput | GenerationReplyInput
    | EditParamsInput | EditRequestInput | EditReplyInput
)
