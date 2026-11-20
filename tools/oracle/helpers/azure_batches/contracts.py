from typing import Literal, NotRequired, TypeAlias, TypedDict

from litellm.types.llms.openai import CreateBatchRequest, RetrieveBatchRequest
from openai import AsyncAzureOpenAI, AsyncOpenAI, AzureOpenAI, OpenAI
from pydantic import JsonValue, TypeAdapter

ENDPOINT = "https://fixture.test"
KEY = "supplied-key"
VERSION = "2024-10-21"
ERROR_STATUS = 400
CLIENT_MISSING = (
    "OpenAI client is not initialized. Make sure api_key is passed "
    "or OPENAI_API_KEY is set in the environment."
)

JSON = TypeAdapter(JsonValue)
VERSION_QUERY = TypeAdapter(str | None)
JsonObject: TypeAlias = dict[str, JsonValue]
Supply: TypeAlias = Literal["openai", "async_openai", "azure_openai", "async_azure_openai"]
SdkClient: TypeAlias = OpenAI | AsyncOpenAI | AzureOpenAI | AsyncAzureOpenAI


class ErrorPayload(TypedDict):
    message: str
    type: NotRequired[str | None]
    param: NotRequired[str | None]
    code: NotRequired[str | None]


class ErrorEnvelope(TypedDict):
    error: ErrorPayload


ENVELOPE = TypeAdapter(ErrorEnvelope)
PAYLOAD = TypeAdapter(ErrorPayload)


class Reply(TypedDict):
    status: int
    body: JsonValue


class Args(TypedDict):
    api_key: str | None
    api_base: str | None
    api_version: str | None
    timeout: float
    max_retries: int | None
    organization: str | None


class Step(TypedDict):
    action: Literal["project", "create", "retrieve", "cancel", "list"]
    args: Args
    is_async: NotRequired[bool]
    client: NotRequired[Supply]
    builder: NotRequired[Literal["default", "none"]]
    data: NotRequired[CreateBatchRequest]
    query: NotRequired[RetrieveBatchRequest]
    params: NotRequired[dict[str, JsonValue]]
    after: NotRequired[str]
    limit: NotRequired[int]
    reply: NotRequired[Reply]


class Input(TypedDict):
    steps: list[Step]
