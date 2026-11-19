import asyncio
from collections.abc import AsyncIterator, Callable, Iterator
from datetime import UTC, datetime
from typing import Final, Literal, NotRequired, Protocol, TypedDict, Unpack, cast

import httpx
import litellm
from litellm.llms.azure.azure import AzureChatCompletion
from litellm.llms.azure.common_utils import AzureOpenAIError
from litellm.llms.custom_httpx.http_handler import HTTPHandler
from litellm.litellm_core_utils.litellm_logging import Logging
from openai import AsyncAzureOpenAI, AsyncOpenAI, AzureOpenAI, OpenAI
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter
from typing_extensions import override

MODEL: Final = "deployment"
API_BASE: Final = "https://unit.test"
API_VERSION: Final = "2024-02-01"
CREATED_FLOOR: Final = 1_000_000_000
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
STATUS: Final[TypeAdapter[int | None]] = TypeAdapter(int | None)
TEXT: Final[TypeAdapter[str | None]] = TypeAdapter(str | None)
OPTIONAL_HEADER_MAP: Final[TypeAdapter[dict[str, str] | None]] = TypeAdapter(
    dict[str, str] | None
)


class Turn(TypedDict):
    role: str
    content: str


class Presence(TypedDict):
    present: bool
    value: NotRequired[JsonValue]


class SendSpec(TypedDict, total=False):
    headers: dict[str, str]
    events: list[str]
    body: str


class ExchangeSpec(TypedDict, total=False):
    environment: dict[str, str]
    status: int
    body: str
    assertion: str


class ErrorSpec(TypedDict, total=False):
    text: str
    status: Presence
    message: Presence
    headers: Presence
    body: Presence
    response: Presence


class ClientSpec(TypedDict, total=False):
    kind: Literal["azure_sync", "azure_async", "open_ai_sync", "open_ai_async"]
    api_key: str


class StreamCase(TypedDict, total=False):
    model: str
    api_base: str
    api_version: str
    api_key: str | None
    azure_ad_token: str | None
    azure_ad_token_provider: int
    timeout: float | None
    max_retries: int
    dynamic_params: bool
    data: dict[str, JsonValue]
    litellm_params: dict[str, JsonValue]
    supplied: ClientSpec
    get_error: ErrorSpec
    send: SendSpec
    exchange: ExchangeSpec


class Input(TypedDict):
    function: Literal["streaming", "async_streaming"]
    cases: list[StreamCase]


class WrapperView(Protocol):
    model: str
    custom_llm_provider: str | None
    stream_options: dict[str, JsonValue] | None
    _response_headers: dict[str, str] | None
    _hidden_params: dict[str, JsonValue]


class ErrorView(Protocol):
    status_code: int | None
    message: str | None
    headers: httpx.Headers | dict[str, str] | None
    body: JsonValue


class PreArgs(BaseModel):
    model_config = ConfigDict(extra="allow")

    headers: dict[str, JsonValue] | None = None
    api_base: str | None = None
    acompletion: bool = False
    complete_input_dict: dict[str, JsonValue] | None = None


PRE_ARGS: Final[TypeAdapter[PreArgs]] = TypeAdapter(PreArgs)


class ChunkDelta(TypedDict, total=False):
    content: str | None
    role: str | None


class ChunkChoice(TypedDict):
    delta: ChunkDelta
    finish_reason: str | None
    index: int


class ChunkView(TypedDict):
    model: str
    created: int
    choices: list[ChunkChoice]


CHUNK: Final[TypeAdapter[ChunkView]] = TypeAdapter(ChunkView)


class JsonDump(Protocol):
    def model_dump_json(self) -> str: ...


class SyncStream(Protocol):
    def __iter__(self) -> Iterator[JsonDump]: ...


class AsyncStream(Protocol):
    def __aiter__(self) -> AsyncIterator[JsonDump]: ...


class StreamKwargs(TypedDict):
    logging_obj: Logging
    api_base: str
    api_key: str | None
    api_version: str
    dynamic_params: bool
    data: dict[str, JsonValue]
    model: str
    timeout: float | None
    max_retries: int
    azure_ad_token: str | None
    azure_ad_token_provider: Callable[[], str] | None
    client: AzureOpenAI | AsyncAzureOpenAI | OpenAI | AsyncOpenAI | None
    litellm_params: dict[str, JsonValue]


class SyncChat(Protocol):
    def streaming(self, **kwargs: Unpack[StreamKwargs]) -> JsonDump: ...


class AsyncChat(Protocol):
    async def async_streaming(self, **kwargs: Unpack[StreamKwargs]) -> JsonDump: ...


class ScriptedError(Exception):
    status_code: int | None
    message: str | None
    headers: httpx.Headers | dict[str, str] | None
    body: JsonValue
    response: httpx.Response | None

    def __init__(self, spec: ErrorSpec) -> None:
        super().__init__(spec["text"])
        status = spec.get("status")
        if status is not None and status["present"]:
            self.status_code = STATUS.validate_python(status.get("value"))
        message = spec.get("message")
        if message is not None and message["present"]:
            self.message = TEXT.validate_python(message.get("value"))
        headers = spec.get("headers")
        if headers is not None and headers["present"]:
            self.headers = OPTIONAL_HEADER_MAP.validate_python(headers.get("value"))
        body = spec.get("body")
        if body is not None and body["present"]:
            self.body = JSON.validate_python(body.get("value"))
        response = spec.get("response")
        if response is not None and response["present"]:
            response_headers = OPTIONAL_HEADER_MAP.validate_python(response.get("value"))
            if response_headers is None:
                self.response = None
            else:
                self.response = httpx.Response(
                    200,
                    headers=response_headers,
                    request=httpx.Request("POST", API_BASE),
                )


class Fixture:
    def __init__(self, send: SendSpec) -> None:
        self.send = send
        self.requests: list[JsonValue] = []

    def __call__(self, request: httpx.Request) -> httpx.Response:
        self.requests.append({"data": JSON.validate_json(request.content)})
        return self.response(request)

    def response(self, request: httpx.Request) -> httpx.Response:
        headers = self.send.get("headers")
        if "headers" not in self.send:
            headers = {"content-type": "application/json"}
        events = self.send.get("events")
        if events is not None:
            return httpx.Response(
                200, headers=headers, content="".join(events).encode(), request=request
            )
        body = self.send.get("body")
        if body is None:
            body = ""
        return httpx.Response(200, headers=headers, content=body.encode(), request=request)


class TokenEndpoint:
    def __init__(self, spec: ExchangeSpec) -> None:
        self.spec = spec
        self.forms: list[JsonValue] = []

    def post(
        self, url: str, data: dict[str, str] | None = None, **kwargs: JsonValue
    ) -> httpx.Response:
        del kwargs
        form: dict[str, str] = {}
        if data is not None:
            form = dict(data)
        self.forms.append({"url": url, "form": form})
        status = self.spec.get("status")
        if status is None:
            status = 200
        body = self.spec.get("body")
        if body is None:
            body = ""
        return httpx.Response(status, content=body.encode())


def scripted_provider() -> str:
    message = "the azure_ad_token_provider is never called by either streaming path"
    raise RuntimeError(message)


def client_for(
    case: StreamCase, fixture: Fixture
) -> AzureOpenAI | AsyncAzureOpenAI | OpenAI | AsyncOpenAI | None:
    supplied = case.get("supplied")
    if supplied is None:
        return None
    kind = supplied.get("kind")
    if kind is None:
        kind = "azure_sync"
    key = supplied.get("api_key")
    version = case.get("api_version")
    if version is None:
        version = API_VERSION
    base = case.get("api_base")
    if base is None:
        base = API_BASE
    transport = httpx.MockTransport(fixture)
    if kind == "azure_sync":
        return AzureOpenAI(
            api_key=key,
            azure_endpoint=base,
            api_version=version,
            max_retries=0,
            http_client=httpx.Client(transport=transport),
        )
    if kind == "azure_async":
        return AsyncAzureOpenAI(
            api_key=key,
            azure_endpoint=base,
            api_version=version,
            max_retries=0,
            http_client=httpx.AsyncClient(transport=transport),
        )
    if kind == "open_ai_sync":
        return OpenAI(
            api_key=key,
            base_url=base,
            max_retries=0,
            http_client=httpx.Client(transport=transport),
        )
    return AsyncOpenAI(
        api_key=key,
        base_url=base,
        max_retries=0,
        http_client=httpx.AsyncClient(transport=transport),
    )


def pre_view(params: dict[str, JsonValue]) -> JsonValue:
    additional = PRE_ARGS.validate_python(params.get("additional_args"))
    return {
        "input": params.get("input"),
        "api_key": params.get("api_key"),
        "headers": additional.headers,
        "api_base": additional.api_base,
        "acompletion": additional.acompletion,
        "complete_input_dict": additional.complete_input_dict,
    }


class Recorder(Logging):
    def __init__(self) -> None:
        super().__init__(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/litellm_logging.py#L576
            model=MODEL,
            messages=None,
            stream=True,
            call_type="completion",
            start_time=datetime(2024, 1, 2, tzinfo=UTC),
            litellm_call_id="call",
            function_id="function",
        )
        self.pre: list[JsonValue] = []

    @override
    def pre_call(self, *args: JsonValue, **params: JsonValue) -> None:
        del args
        self.pre.append(pre_view(params))


def stored_headers(recorder: Recorder) -> JsonValue:
    details: dict[str, JsonValue] = recorder.model_call_details
    return JSON.validate_python(details.get("response_headers"))


def wrapper_view(result: JsonDump) -> JsonValue:
    view = cast(WrapperView, cast(object, result))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/streaming_handler.py#L208-L299
    options = view.stream_options
    if options is None:
        stream_options: JsonValue = None
    else:
        stream_options = {"include_usage": options.get("include_usage") is True}
    return {
        "model": view.model,
        "provider": view.custom_llm_provider,
        "stream_options": stream_options,
        "response_headers": JSON.validate_python(view._response_headers),  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/streaming_handler.py#L208-L299
        "additional_headers": JSON.validate_python(view._hidden_params.get("additional_headers")),  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/streaming_handler.py#L208-L299
    }


def chunk_view(view: ChunkView) -> JsonValue:
    choice = view["choices"][0]
    return {
        "content": choice["delta"].get("content"),
        "role": choice["delta"].get("role"),
        "finish_reason": choice["finish_reason"],
        "index": choice["index"],
        "model": view["model"],
        "created_is_current": view["created"] > CREATED_FLOOR,
    }


async def collect(result: JsonDump, *, asynchronous: bool) -> list[JsonValue]:
    if asynchronous:
        stream = cast(AsyncStream, cast(object, result))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/streaming_handler.py#L1978-L1990
        return [
            chunk_view(CHUNK.validate_json(chunk.model_dump_json())) async for chunk in stream
        ]
    buffered = cast(SyncStream, cast(object, result))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/streaming_handler.py#L1768-L1775
    return [chunk_view(CHUNK.validate_json(chunk.model_dump_json())) for chunk in buffered]


def header_map(headers: httpx.Headers | dict[str, str] | None) -> JsonValue:
    if headers is None:
        return None
    return dict(headers.items())


def azure_error_view(error: AzureOpenAIError) -> JsonValue:
    view = cast(ErrorView, cast(object, error))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L46-L63
    return {
        "class": type(error).__name__,
        "status_present": True,
        "status": view.status_code,
        "message_present": True,
        "message": view.message,
        "headers": header_map(view.headers),
        "body": JSON.validate_python(view.body),
    }


def scripted_status(error: ScriptedError) -> tuple[JsonValue, bool]:
    try:
        value: JsonValue = error.status_code
    except AttributeError:
        return None, False
    else:
        return value, True


def scripted_message(error: ScriptedError) -> tuple[JsonValue, bool]:
    try:
        value: JsonValue = error.message
    except AttributeError:
        return None, False
    else:
        return value, True


def scripted_headers(error: ScriptedError) -> JsonValue:
    try:
        value = header_map(error.headers)
    except AttributeError:
        return None
    else:
        return value


def scripted_body(error: ScriptedError) -> JsonValue:
    try:
        value = JSON.validate_python(error.body)
    except AttributeError:
        return None
    else:
        return value


def scripted_error_view(error: ScriptedError) -> JsonValue:
    status, status_present = scripted_status(error)
    message, message_present = scripted_message(error)
    return {
        "class": type(error).__name__,
        "status_present": status_present,
        "status": status,
        "message_present": message_present,
        "message": message,
        "headers": scripted_headers(error),
        "body": scripted_body(error),
    }


class ScriptedChatCompletion(AzureChatCompletion):
    get_error: ErrorSpec | None
    gets: list[JsonValue]

    def __init__(self, get_error: ErrorSpec | None) -> None:
        super().__init__()
        self.get_error = get_error
        self.gets = []

    @override
    def get_azure_openai_client(  # pyright: ignore[reportIncompatibleMethodOverride]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L480-L489
        self,
        api_key: str | None,
        api_base: str | None,
        api_version: str | None = None,
        client: AzureOpenAI | AsyncAzureOpenAI | OpenAI | AsyncOpenAI | None = None,
        litellm_params: dict[str, JsonValue] | None = None,
        _is_async: bool = False,
        model: str | None = None,
    ) -> AzureOpenAI | AsyncAzureOpenAI | OpenAI | AsyncOpenAI | None:
        if self.get_error is not None:
            raise ScriptedError(self.get_error)
        params: dict[str, JsonValue] = {} if litellm_params is None else litellm_params
        self.gets.append(
            {
                "api_version": api_version,
                "api_base": api_base,
                "api_key": api_key,
                "model": model,
                "is_async": _is_async,
                "litellm_params": {
                    "azure_ad_token": params.get("azure_ad_token"),
                    "max_retries": params.get("max_retries"),
                    "timeout": params.get("timeout"),
                },
            }
        )
        return client


def data_of(case: StreamCase) -> dict[str, JsonValue]:
    data = case.get("data")
    if data is None:
        return {}
    return dict(data)


def litellm_params_of(case: StreamCase) -> dict[str, JsonValue]:
    params = case.get("litellm_params")
    if params is None:
        return {}
    return params


def stream_kwargs(
    case: StreamCase,
    recorder: Recorder,
    client: AzureOpenAI | AsyncAzureOpenAI | OpenAI | AsyncOpenAI | None,
) -> StreamKwargs:
    provider: Callable[[], str] | None = None
    if "azure_ad_token_provider" in case:
        provider = scripted_provider
    return StreamKwargs(
        logging_obj=recorder,
        api_base=case["api_base"],
        api_key=case.get("api_key"),
        api_version=case["api_version"],
        dynamic_params=bool(case.get("dynamic_params")),
        data=data_of(case),
        model=case["model"],
        timeout=case.get("timeout"),
        max_retries=case["max_retries"],
        azure_ad_token=case.get("azure_ad_token"),
        azure_ad_token_provider=provider,
        client=client,
        litellm_params=litellm_params_of(case),
    )


class CallContext(TypedDict):
    handler: ScriptedChatCompletion
    recorder: Recorder
    fixture: Fixture
    endpoint: TokenEndpoint


def call_view(
    context: CallContext,
    outcome: str,
    wrapper: JsonValue,
    chunks: JsonValue,
    error: JsonValue,
) -> JsonValue:
    return {
        "outcome": outcome,
        "gets": context["handler"].gets,
        "requests": context["fixture"].requests,
        "exchanges": context["endpoint"].forms,
        "pre": context["recorder"].pre,
        "response_headers": stored_headers(context["recorder"]),
        "wrapper": wrapper,
        "chunks": chunks,
        "error": error,
    }


async def stream_call(function: str, case: StreamCase) -> JsonValue:
    send = case.get("send")
    fixture = Fixture({} if send is None else send)
    exchange = case.get("exchange")
    endpoint = TokenEndpoint({} if exchange is None else exchange)
    client = client_for(case, fixture)
    recorder = Recorder()
    handler = ScriptedChatCompletion(case.get("get_error"))
    context = CallContext(
        handler=handler, recorder=recorder, fixture=fixture, endpoint=endpoint
    )
    kwargs = stream_kwargs(case, recorder, client)
    sync_handler = cast(SyncChat, cast(object, handler))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/azure.py#L500-L575
    async_handler = cast(AsyncChat, cast(object, handler))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/azure.py#L577-L652
    asynchronous = function == "async_streaming"
    # get_azure_ad_token_from_oidc posts through litellm.module_level_client
    litellm.module_level_client = cast(HTTPHandler, cast(object, endpoint))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L249
    result: JsonDump
    try:
        if asynchronous:
            result = await async_handler.async_streaming(**kwargs)
        else:
            result = sync_handler.streaming(**kwargs)
    except ScriptedError as error:
        return call_view(context, "error", None, None, scripted_error_view(error))
    except AzureOpenAIError as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L46
        return call_view(context, "error", None, None, azure_error_view(error))
    wrapper = wrapper_view(result)
    chunks = await collect(result, asynchronous=asynchronous)
    return call_view(context, "stream", wrapper, chunks, None)


async def run_cases(input_value: Input) -> JsonValue:
    calls = [await stream_call(input_value["function"], case) for case in input_value["cases"]]
    return {"calls": calls}


def run(input_value: Input) -> JsonValue:
    litellm.suppress_debug_info = True
    return asyncio.run(run_cases(input_value))
