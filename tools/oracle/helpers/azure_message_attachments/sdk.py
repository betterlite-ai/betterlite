import asyncio
import os
from collections.abc import Coroutine, Generator
from contextlib import contextmanager
from dataclasses import dataclass, field
from typing import Literal, Protocol, TypeAlias, TypedDict, cast

import httpx
import litellm
import openai
import pydantic
from litellm.llms.azure.assistants import AzureAssistantsAPI
from litellm.llms.azure.azure import AzureOpenAIAssistantsAPIConfig
from openai import AsyncAzureOpenAI, AzureOpenAI
from pydantic import JsonValue, TypeAdapter

JSON = TypeAdapter(JsonValue)

MESSAGE = """{"id":"msg_1","object":"thread.message","created_at":1,"thread_id":"thread_1",
"role":"user","content":[{"type":"text","text":{"value":"hi","annotations":[]}}],"status":"completed"}"""
PAGE = '{"object":"list","data":[],"first_id":null,"last_id":null,"has_more":false}'
DEFAULT_TIMEOUT = 600.0
THREAD_ID = "thread_1"
SUPPLIED_ENDPOINT = "https://supplied.test"
SUPPLIED_VERSION = "2023-05-01"
SUPPLIED_CREDENTIAL = "supplied-key"
SUPPLIED_TIMEOUT = 42.0
SUPPLIED_RETRIES = 0

ASSISTANTS = AzureAssistantsAPI()
CONFIG = AzureOpenAIAssistantsAPIConfig()

Result: TypeAlias = pydantic.BaseModel | Coroutine[None, None, pydantic.BaseModel]
SdkClient: TypeAlias = AzureOpenAI | AsyncAzureOpenAI
Kind: TypeAlias = Literal["sync", "async"]
Params: TypeAlias = dict[str, JsonValue]


class Reply(TypedDict):
    status: int
    body: str


class Supplied(TypedDict):
    kind: Kind


class Step(TypedDict, total=False):
    params: Params
    api_key: str
    api_base: str
    api_version: str
    azure_ad_token: str
    timeout: float
    max_retries: int
    environment: dict[str, str]
    flag: JsonValue
    thread_id: str
    data: Params
    supplied: Supplied
    reply: Reply


class MapStep(TypedDict, total=False):
    non_default: Params
    optional: Params
    drop_params: bool


class Input(TypedDict, total=False):
    append_message: list[Step]
    list_messages: list[Step]
    map_create_message_params: list[MapStep]


class Call(Protocol):
    def __call__(self, *, client: SdkClient | None, **params: JsonValue) -> Result: ...


class SupportedCall(Protocol):
    def __call__(self) -> list[JsonValue]: ...


class MapCall(Protocol):
    def __call__(self, non_default: Params, optional: Params) -> Params: ...


ADD: Call = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/assistants.py#L269-L318
    Call,
    ASSISTANTS.add_message,
)
GET_MESSAGES: Call = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/assistants.py#L383-L421
    Call,
    ASSISTANTS.get_messages,
)
SUPPORTED: SupportedCall = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/azure.py#L71-L77
    SupportedCall,
    CONFIG.get_supported_openai_create_message_params,
)
MAP: MapCall = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/azure.py#L79-L112
    MapCall,
    CONFIG.map_openai_params_create_message_params,
)


@dataclass
class Values:
    api_key: str | None = None
    api_base: str | None = None
    api_version: str | None = None
    azure_ad_token: str | None = None
    timeout: float = DEFAULT_TIMEOUT
    max_retries: int | None = None
    litellm_params: Params = field(default_factory=dict)
    flag: JsonValue = None
    thread_id: str = THREAD_ID


class Wire:
    def __init__(self, canned: str, reply: Reply | None) -> None:
        self.canned = canned
        self.reply = reply
        self.seen: list[JsonValue] = []
        self.result: JsonValue = None
        self.error: JsonValue = None

    def __call__(self, request: httpx.Request) -> httpx.Response:
        self.seen.append(record(request))
        if self.reply is None:
            body = self.canned
            status = 200
        else:
            body = self.reply["body"]
            status = self.reply["status"]
        return httpx.Response(
            status,
            content=body,
            headers={"content-type": "application/json"},
        )


def record(request: httpx.Request) -> JsonValue:
    body: JsonValue = None
    if request.content:
        body = JSON.validate_json(request.content)
    headers = {name.lower(): value for name, value in request.headers.multi_items()}
    recorded: dict[str, JsonValue] = {
        "method": request.method,
        "url": str(request.url),
        "api-key": headers.get("api-key"),
        "authorization": headers.get("authorization"),
        "openai-beta": headers.get("openai-beta"),
        "accept": headers.get("accept"),
        "content-type": headers.get("content-type"),
        "body": body,
    }
    return recorded


def values_of(step: Step) -> Values:
    values = Values()
    if "api_key" in step:
        values.api_key = step["api_key"]
    if "api_base" in step:
        values.api_base = step["api_base"]
    if "api_version" in step:
        values.api_version = step["api_version"]
    if "azure_ad_token" in step:
        values.azure_ad_token = step["azure_ad_token"]
    if "timeout" in step:
        values.timeout = float(step["timeout"])
    if "max_retries" in step:
        values.max_retries = step["max_retries"]
    if "flag" in step:
        values.flag = step["flag"]
    if "thread_id" in step:
        values.thread_id = step["thread_id"]
    if "params" in step:
        values.litellm_params = dict(step["params"])
    return values


def params_of(values: Values) -> Params:
    params: Params = {
        "thread_id": values.thread_id,
        "api_key": values.api_key,
        "api_base": values.api_base,
        "api_version": values.api_version,
        "azure_ad_token": values.azure_ad_token,
        "timeout": values.timeout,
        "max_retries": values.max_retries,
        "litellm_params": values.litellm_params,
    }
    return params


def message_data(step: Step) -> Params:
    if "data" not in step:
        raise ValueError("an append step carries no data")
    return step["data"]


def supplied_client(step: Step, wire: Wire) -> SdkClient | None:
    if "supplied" not in step:
        return None
    supplied = step["supplied"]
    if supplied["kind"] == "async":
        return AsyncAzureOpenAI(
            azure_endpoint=SUPPLIED_ENDPOINT,
            api_version=SUPPLIED_VERSION,
            api_key=SUPPLIED_CREDENTIAL,
            timeout=SUPPLIED_TIMEOUT,
            max_retries=SUPPLIED_RETRIES,
            http_client=httpx.AsyncClient(transport=httpx.MockTransport(wire)),
        )
    return AzureOpenAI(
        azure_endpoint=SUPPLIED_ENDPOINT,
        api_version=SUPPLIED_VERSION,
        api_key=SUPPLIED_CREDENTIAL,
        timeout=SUPPLIED_TIMEOUT,
        max_retries=SUPPLIED_RETRIES,
        http_client=httpx.Client(transport=httpx.MockTransport(wire)),
    )


def kind_of(step: Step, flag: JsonValue) -> Kind:
    if "supplied" in step:
        return step["supplied"]["kind"]
    if flag is True:
        return "async"
    return "sync"


def settle(result: Result) -> JsonValue:
    match result:
        case Coroutine():
            with asyncio.Runner() as runner:
                settled = runner.run(result)
            return JSON.validate_json(settled.model_dump_json())
        case object():
            return JSON.validate_json(result.model_dump_json())


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


@contextmanager
def reported(wire: Wire) -> Generator[None, None, None]:
    try:
        yield
    except openai.APIStatusError as error:
        wire.error = {"kind": "status", "status": error.status_code}
    except openai.OpenAIError:
        wire.error = {"kind": "configuration"}
    except ValueError:
        wire.error = {"kind": "value"}


def report(wire: Wire, kind: Kind) -> JsonValue:
    values: dict[str, JsonValue] = {"requests": wire.seen}
    if wire.error is None:
        values["kind"] = kind
        values["result"] = wire.result
    else:
        values["error"] = wire.error
    return values
