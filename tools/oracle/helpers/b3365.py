import asyncio
import datetime as dt
from collections.abc import Awaitable, Coroutine
from typing import Final, Literal, TypeAlias, TypedDict, TypeVar

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.llms.base_llm.containers.transformation import BaseContainerConfig
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler, HTTPHandler
from litellm.llms.custom_httpx.llm_http_handler import BaseLLMHTTPHandler
from litellm.types.containers.main import ContainerObject, DeleteContainerResult
from litellm.types.router import GenericLiteLLMParams
from litellm.types.utils import CallTypes, LlmProviders
from litellm.utils import ProviderConfigManager
from pydantic import BaseModel, JsonValue, TypeAdapter
from typing_extensions import override

litellm.suppress_debug_info = True

# LiteLLM's class carries Any in its constructor, so the catch names it through a typed alias
PROVIDER_ERROR: Final[type[Exception]] = BaseLLMException
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
# LiteLLM takes the extra maps as dict[str, object], so the case's JSON maps bind through this alias
ExtraMaps: TypeAlias = dict[str, object] | None
OBJECTS: Final = TypeAdapter(ExtraMaps)
HANDLER: Final = BaseLLMHTTPHandler()
STARTED: Final = dt.datetime(2020, 1, 1, tzinfo=dt.UTC)


class Reply(TypedDict):
    status: int
    headers: dict[str, str]
    body: str


class Input(TypedDict):
    operation: Literal["retrieve", "delete"]
    asynchronous: bool
    container_id: str
    api_key: str
    api_base: str
    extra_headers: dict[str, str]
    extra_query: dict[str, str]
    reply: Reply
    failure: Literal["answer", "timeout"]


class LoggedCall(TypedDict):
    api_base: str
    headers: dict[str, str]
    params: dict[str, JsonValue]
    container_id: str


LOGGED: Final = TypeAdapter(LoggedCall)


class ErrorRecord(BaseModel):
    status_code: int
    message: str
    headers: dict[str, str]
    status_code_is_synthesized: bool


class RecordingLogging(Logging):
    logged: JsonValue = None

    @override
    def pre_call(self, *args: JsonValue, **params: JsonValue) -> None:
        self.logged = params["additional_args"]


class Scripted:
    def __init__(self, reply: Reply, failure: str) -> None:
        self.reply = reply
        self.failure = failure
        self.seen: httpx.Request | None = None

    def __call__(self, request: httpx.Request) -> httpx.Response:
        self.seen = request
        if self.failure == "timeout":
            raise httpx.ReadTimeout("read timed out", request=request)
        return httpx.Response(
            self.reply["status"],
            headers=self.reply["headers"],
            content=self.reply["body"].encode(),
        )


Client: TypeAlias = HTTPHandler | AsyncHTTPHandler
Record = TypeVar("Record", ContainerObject, DeleteContainerResult)


class HandlerArguments(TypedDict):
    container_id: str
    container_provider_config: BaseContainerConfig
    litellm_params: GenericLiteLLMParams
    logging_obj: Logging
    extra_headers: ExtraMaps
    extra_query: ExtraMaps
    timeout: float
    _is_async: bool
    client: Client


def container_config() -> BaseContainerConfig:
    found = ProviderConfigManager.get_provider_container_config(LlmProviders.OPENAI)
    if found is None:
        raise ValueError("LiteLLM registers no OpenAI container config")
    return found


def build_client(recorder: Scripted, *, asynchronous: bool) -> Client:
    wire = httpx.MockTransport(recorder)
    if asynchronous:
        return AsyncHTTPHandler(transport=wire)
    return HTTPHandler(client=httpx.Client(transport=wire))


def make_logging() -> RecordingLogging:
    return RecordingLogging(
        model="openai/container",
        messages=None,
        stream=False,
        call_type=CallTypes.create_container,
        start_time=STARTED,
        litellm_call_id="b3365",
        function_id="b3365",
    )


def settle(answer: Record | Awaitable[Record]) -> Record:
    match answer:
        case Coroutine():
            with asyncio.Runner() as runner:
                return runner.run(answer)
        case object():
            return answer


def container_arguments(call: Input, logging: Logging, client: Client) -> HandlerArguments:
    return {
        "container_id": call["container_id"],
        "container_provider_config": container_config(),
        "litellm_params": GenericLiteLLMParams(
            api_key=call["api_key"], api_base=call["api_base"]
        ),
        "logging_obj": logging,
        "extra_headers": OBJECTS.validate_python(call["extra_headers"]),
        "extra_query": OBJECTS.validate_python(call["extra_query"]),
        "timeout": 600,
        "_is_async": call["asynchronous"],
        "client": client,
    }


def container_result(call: Input, logging: Logging, client: Client) -> JsonValue:
    arguments = container_arguments(call, logging, client)
    if call["operation"] == "retrieve":
        retrieved = settle(HANDLER.container_retrieve_handler(**arguments))
        return ok_outcome(container_json(retrieved))
    deleted = settle(HANDLER.container_delete_handler(**arguments))
    return ok_outcome(deletion_json(deleted))


def ok_outcome(record: JsonValue) -> JsonValue:
    outcome: dict[str, JsonValue] = {"kind": "ok", "record": record}
    return JSON.validate_python(outcome)


def container_json(record: ContainerObject) -> JsonValue:
    fields: dict[str, JsonValue] = {
        "id": record.id,
        "object": record.object,
        "created_at": record.created_at,
        "status": record.status,
        "name": record.name,
    }
    return JSON.validate_python(fields)


def deletion_json(record: DeleteContainerResult) -> JsonValue:
    fields: dict[str, JsonValue] = {
        "id": record.id,
        "object": record.object,
        "deleted": record.deleted,
    }
    return JSON.validate_python(fields)


def lower_headers(headers: dict[str, str], names: set[str]) -> dict[str, JsonValue]:
    kept: dict[str, JsonValue] = {}
    for name, value in headers.items():
        if name.lower() in names:
            kept[name.lower()] = value
    return kept


def error_outcome(error: Exception, reply: Reply) -> JsonValue:
    record = ErrorRecord.model_validate(error, from_attributes=True)
    names = {name.lower() for name in reply["headers"]}
    outcome: dict[str, JsonValue] = {
        "kind": "provider",
        "status": record.status_code,
        "synthesized": record.status_code_is_synthesized,
        "message": record.message,
        "headers": lower_headers(record.headers, names),
    }
    return JSON.validate_python(outcome)


def logged_json(logging: RecordingLogging, names: set[str]) -> JsonValue:
    call = LOGGED.validate_python(logging.logged)
    outcome: dict[str, JsonValue] = {
        "api_base": call["api_base"],
        "headers": lower_headers(call["headers"], names),
        "params": call["params"],
        "container_id": call["container_id"],
    }
    return JSON.validate_python(outcome)


def run(input_value: Input) -> JsonValue:
    recorder = Scripted(input_value["reply"], input_value["failure"])
    client = build_client(recorder, asynchronous=input_value["asynchronous"])
    logging = make_logging()
    outcome: JsonValue = None
    try:
        outcome = container_result(input_value, logging, client)
    except PROVIDER_ERROR as error:
        outcome = error_outcome(error, input_value["reply"])
    request = recorder.seen
    if request is None:
        raise ValueError("the handler sent no request")
    names = {"authorization"} | {name.lower() for name in input_value["extra_headers"]}
    observation: dict[str, JsonValue] = {
        "method": request.method,
        "url": str(request.url),
        "headers": lower_headers(dict(request.headers.items()), names),
        "logged": logged_json(logging, names),
        "outcome": outcome,
    }
    return JSON.validate_python(observation)
