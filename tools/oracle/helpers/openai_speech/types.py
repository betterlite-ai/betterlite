from typing import Literal, NotRequired, TypeAlias, TypedDict

import openai
from litellm.types.llms.openai import HttpxBinaryResponseContent

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Fields: TypeAlias = dict[str, Json]
CallOutcome: TypeAlias = tuple[
    HttpxBinaryResponseContent | None,
    openai.APIStatusError | None,
    openai.OpenAI | openai.AsyncOpenAI,
]


class ClientSpec(TypedDict):
    timeout: int
    max_retries: int
    api_key: NotRequired[str]
    api_base: NotRequired[str]
    organization: NotRequired[str]
    project: NotRequired[str]
    environment_organization: NotRequired[str]
    environment_project: NotRequired[str]
    aspeech: NotRequired[bool]
    supplied: NotRequired[Literal["sync", "async"]]
    shared_session: NotRequired[bool]


class Request(TypedDict):
    model: str
    input: str
    voice: str
    response_format: NotRequired[str]
    speed: NotRequired[float]
    instructions: NotRequired[str]
    extra_headers: NotRequired[dict[str, Json]]


class Reply(TypedDict, total=False):
    status: int
    headers: list[list[str]]
    chunks: list[list[int]]
    text: str


class Input(TypedDict):
    client: ClientSpec
    request: Request
    reply: NotRequired[Reply]
