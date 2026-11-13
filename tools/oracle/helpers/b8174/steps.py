from typing import NotRequired, TypedDict

from pydantic import JsonValue

Object = dict[str, JsonValue]


class StepRequired(TypedDict):
    kind: str


class Step(StepRequired, total=False):
    model: str
    input: str
    voice: str | dict[str, JsonValue]
    params: Object
    api_base: str
    env_base: str
    api_key: str
    global_key: str
    env_key: str
    status: int
    headers: dict[str, str]
    body: str
    drop_params: bool
    prompt: str
    created: int


class Input(TypedDict):
    steps: list[Step]


class SpeechParamsStep(TypedDict, total=False):
    voice: str | dict[str, JsonValue]
    params: Object


class SpeechBodyStep(TypedDict):
    model: str
    input: str
    voice: NotRequired[str | dict[str, JsonValue]]
    params: NotRequired[Object]


class UrlStep(TypedDict, total=False):
    api_base: str
    env_base: str


class SpeechHeadersStep(TypedDict, total=False):
    api_key: str
    global_key: str
    env_key: str


class SpeechReplyStep(TypedDict):
    status: int
    body: str
    headers: NotRequired[dict[str, str]]


class ImageReplyStep(TypedDict):
    status: int
    body: str
    created: NotRequired[int]


class ImageParamsStep(TypedDict):
    model: str
    params: NotRequired[Object]
    drop_params: NotRequired[bool]


class ImageBodyStep(TypedDict):
    model: str
    prompt: str
    params: NotRequired[Object]
