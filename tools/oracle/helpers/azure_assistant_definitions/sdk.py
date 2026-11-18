import asyncio
import os
from collections.abc import Coroutine, Generator
from contextlib import contextmanager
from dataclasses import dataclass
from typing import Literal, Protocol, TypeAlias, TypedDict, cast

import httpx
import litellm
import pydantic
from litellm.llms.azure.assistants import AzureAssistantsAPI
from openai import AsyncAzureOpenAI, AzureOpenAI
from pydantic import JsonValue, TypeAdapter

JSON = TypeAdapter(JsonValue)

PAGE = '{"object":"list","data":[],"first_id":null,"last_id":null,"has_more":false}'
ASSISTANT = '{"id":"asst_1","object":"assistant","created_at":1,"model":"gpt-4o","tools":[]}'
DELETED = '{"id":"asst_1","deleted":true,"object":"assistant.deleted"}'
DEFAULT_TIMEOUT = 600.0
SUPPLIED_BASE_URL = "https://supplied.test/openai"
SUPPLIED_TIMEOUT = 42.0

ASSISTANTS = AzureAssistantsAPI()

Result: TypeAlias = pydantic.BaseModel | Coroutine[None, None, pydantic.BaseModel]
SdkClient: TypeAlias = AzureOpenAI | AsyncAzureOpenAI


class Reply(TypedDict):
    status: int
    body: str


class Settled(TypedDict):
    kind: str
    result: JsonValue


class Supplied(TypedDict):
    kind: Literal["sync", "async"]


class Step(TypedDict, total=False):
    params: dict[str, JsonValue]
    api_key: str
    api_base: str
    api_version: str
    azure_ad_token: str
    timeout: float
    max_retries: int
    environment: dict[str, str]
    flag: JsonValue
    data: dict[str, JsonValue]
    assistant_id: str
    kind: Literal["sync", "async"]
    supplied: Supplied
    reply: Reply


class Input(TypedDict, total=False):
    list_assistants: list[Step]
    create_assistant: list[Step]
    delete_assistant: list[Step]
    client_arguments: list[Step]


class Call(Protocol):
    def __call__(self, **params: JsonValue) -> Result: ...


class ClientCall(Protocol):
    def __call__(
        self, *, client: SdkClient | None, **params: JsonValue
    ) -> SdkClient: ...


LIST: Call = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/assistants.py#L125-L190
    Call,
    ASSISTANTS.get_assistants,
)
CREATE: Call = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/assistants.py#L934-L971
    Call,
    ASSISTANTS.create_assistants,
)
DELETE: Call = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/assistants.py#L1000-L1037
    Call,
    ASSISTANTS.delete_assistant,
)
GET_CLIENT: ClientCall = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/assistants.py#L40-L64
    ClientCall,
    ASSISTANTS.get_azure_client,
)
ASYNC_GET_CLIENT: ClientCall = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/assistants.py#L66-L92
    ClientCall,
    ASSISTANTS.async_get_azure_client,
)


@dataclass
class Values:
    api_key: str | None
    api_base: str | None
    api_version: str | None
    azure_ad_token: str | None
    timeout: float
    max_retries: int | None
    litellm_params: dict[str, JsonValue]
    flag: JsonValue


class SuppliedMark:
    def __init__(self, kind: Literal["sync", "async"]) -> None:
        self.kind = kind
        self.base_url = SUPPLIED_BASE_URL
        self.timeout = SUPPLIED_TIMEOUT
        self.max_retries: int | None = None


class Wire:
    def __init__(self, canned: str, reply: Reply | None) -> None:
        self.canned = canned
        self.reply = reply
        self.seen: list[JsonValue] = []
        self.kind: JsonValue = None
        self.result: JsonValue = None
        self.error: JsonValue = None

    def __call__(self, request: httpx.Request) -> httpx.Response:
        self.seen.append(record(request))
        if self.reply is None:
            reply = self.canned
            status = 200
        else:
            reply = self.reply["body"]
            status = self.reply["status"]
        return httpx.Response(
            status,
            content=reply,
            headers={"content-type": "application/json"},
        )


def record(request: httpx.Request) -> JsonValue:
    body: JsonValue = None
    if request.content:
        body = JSON.validate_json(request.content)
    recorded: dict[str, JsonValue] = {
        "method": request.method,
        "url": str(request.url),
        "api-key": request.headers.get("api-key"),
        "authorization": request.headers.get("authorization"),
        "openai-beta": request.headers.get("openai-beta"),
        "accept": request.headers.get("accept"),
        "content-type": request.headers.get("content-type"),
        "body": body,
    }
    return recorded


@contextmanager
def session(step: Step, canned: str) -> Generator[Wire, None, None]:
    reply: Reply | None = None
    if "reply" in step:
        reply = step["reply"]
    wire = Wire(canned, reply)
    environment: dict[str, str] = {}
    if "environment" in step:
        environment = step["environment"]
    saved = dict(os.environ)
    os.environ.clear()
    os.environ.update(environment)
    sync_client = httpx.Client(transport=httpx.MockTransport(wire))
    async_client = httpx.AsyncClient(transport=httpx.MockTransport(wire))
    litellm.client_session = sync_client
    litellm.aclient_session = async_client
    try:
        yield wire
    finally:
        litellm.client_session = None
        litellm.aclient_session = None
        sync_client.close()
        asyncio.run(async_client.aclose())
        os.environ.clear()
        os.environ.update(saved)
