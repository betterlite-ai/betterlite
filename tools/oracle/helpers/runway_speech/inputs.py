from typing import Literal, TypeAlias, TypedDict

from pydantic import JsonValue

Headers: TypeAlias = dict[str, str]
Object: TypeAlias = dict[str, JsonValue]
Voice: TypeAlias = str | dict[str, JsonValue] | None
EMPTY: dict[str, JsonValue] = {}


class SpeechRequestData(TypedDict, total=False):
    dict_body: dict[str, JsonValue]
    ssml_body: str
    headers: dict[str, str]


class Globals(TypedDict):
    api_base: str | None
    api_key: str | None


class MappingCall(TypedDict):
    voice: Voice
    optional_params: Object
    drop_params: bool


class MappingParams(TypedDict):
    model: str
    environment: dict[str, str]
    calls: list[MappingCall]


class MappingInput(TypedDict):
    operation: Literal["mapping"]
    params: MappingParams


class RequestParams(TypedDict):
    model: str
    input: str
    optional_params: Object
    headers: Headers


class RequestInput(TypedDict):
    operation: Literal["request"]
    params: RequestParams


class DispatchParams(TypedDict):
    model: str
    input: str
    voice: Voice
    optional_params: Object
    litellm_params: Object
    api_base: str | None
    api_key: str | None
    timeout_seconds: float
    extra_headers: Object | None
    aspeech: bool
    environment: dict[str, str]
    globals: Globals


class DispatchInput(TypedDict):
    operation: Literal["dispatch"]
    params: DispatchParams


class Submission(TypedDict):
    status: int
    headers: Headers
    body: str


class Reply(TypedDict):
    status: int
    headers: Headers
    body: str


class ResponseParams(TypedDict):
    model: str
    environment: dict[str, str]
    submission: Submission
    request_headers: Headers
    step_millis: int
    replies: list[Reply]


class ResponseInput(TypedDict):
    operation: Literal["response"]
    params: ResponseParams


Input: TypeAlias = MappingInput | RequestInput | DispatchInput | ResponseInput
