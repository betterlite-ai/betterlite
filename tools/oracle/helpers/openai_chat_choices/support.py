import time
import uuid
from typing import TypeAlias, TypedDict, cast
from uuid import UUID

import litellm

litellm.suppress_debug_info = True


Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]


class Step(TypedDict, total=False):
    optional_params: Object | None
    json_mode: bool
    choices: list[Json]


class ChunkStep(TypedDict):
    chunk: Object


class ScriptReply(TypedDict):
    status: int
    body: str


class ModelStep(TypedDict, total=False):
    api_base: str | None
    api_key: str | None
    env: dict[str, str]
    reply: ScriptReply


class UrlStep(TypedDict, total=False):
    api_base: str | None


class AuthStep(TypedDict, total=False):
    api_key: str | None
    headers: dict[str, str]


class ResponseStep(TypedDict):
    status: int
    raw_text: str
    raw_headers: dict[str, str]
    messages: Json
    request_data: Object


class Fields(TypedDict, total=False):
    steps: list[Json]
    api_key: str | None
    litellm_api_key: str | None
    litellm_openai_key: str | None
    api_base: str | None
    litellm_api_base: str | None
    env: dict[str, str]
    model: str


class FixedUuid:
    def __call__(self) -> UUID:
        return UUID(int=0)


class FixedClock:
    def __call__(self) -> int:
        return 7


uuid.uuid4 = FixedUuid()  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L123
time.time = FixedClock()  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2135


def as_object(value: Json) -> Object:
    return cast(Object, value)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/chat/gpt_transformation.py#L864-L866
