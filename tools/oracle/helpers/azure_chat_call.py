import asyncio
from collections.abc import Callable
from datetime import UTC, datetime
from typing import Final, Literal, Protocol, TypeAlias, TypedDict, Unpack, cast

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.azure.azure import AzureChatCompletion
from litellm.llms.azure.chat.o_series_handler import AzureOpenAIO1ChatCompletion
from litellm.llms.azure.common_utils import AzureOpenAIError
from litellm.llms.openai.openai import OpenAIChatCompletion
from litellm.types.utils import ModelResponse
from openai import (
    APIError,
    APIResponseValidationError,
    APIStatusError,
    APITimeoutError,
    AsyncAzureOpenAI,
    AsyncOpenAI,
    AzureOpenAI,
    OpenAI,
    OpenAIError,
)
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter, ValidationError
from typing_extensions import override

litellm.suppress_debug_info = True

API_BASE: Final = "https://unit.test"
API_VERSION: Final = "2024-02-01"
MODEL: Final = "gpt-4o-deployment"
TIMEOUT: Final = 30.0
TIMEOUT_DIAGNOSTIC: Final = "- timeout value="
CHAT_BODY: Final = (
    '{"id":"chatcmpl-1","object":"chat.completion","created":1700000000,'
    '"model":"gpt-4o","choices":[{"index":0,'
    '"message":{"role":"assistant","content":"Hello!"},"finish_reason":"stop"}],'
    '"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}}'
)
PIN: Final = "https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/"
KINDS: Final[dict[str, str]] = {
    "AzureOpenAI": "azure_sync",
    "AsyncAzureOpenAI": "azure_async",
    "OpenAI": "openai_sync",
    "AsyncOpenAI": "openai_async",
}

AzureClient: TypeAlias = AzureOpenAI | AsyncAzureOpenAI
AnyClient: TypeAlias = AzureOpenAI | AsyncAzureOpenAI | OpenAI | AsyncOpenAI


class Turn(TypedDict, total=False):
    role: str
    content: str


class Answer(TypedDict, total=False):
    error: Literal["status", "bad_request", "timeout", "other", "pending"]
    status: int
    message: str
    headers: dict[str, str]
    body: str


class CallCase(TypedDict, total=False):
    model: str | None
    messages: list[Turn] | None
    api_key: str | None
    client_api_key: str | None
    azure_ad_token: str | None
    api_base: str
    api_version: str
    timeout: float
    optional_params: dict[str, JsonValue]
    litellm_params: dict[str, JsonValue]
    base_model: str | None
    organization: str | None
    drop_params: bool | None
    acompletion: bool
    client: Literal["sync", "async"]
    supplied: bool
    headers: dict[str, str] | None
    default_max_retries: int
    openai_system_messages_first: bool
    cancel: bool
    answer: Answer


class Input(TypedDict):
    function: Literal["completion", "o_series"]
    cases: list[CallCase]


class CompletionKwargs(TypedDict):
    model: str | None
    messages: list[Turn] | None
    model_response: ModelResponse
    api_key: str | None
    api_base: str
    api_version: str
    api_type: str
    azure_ad_token: str | None
    azure_ad_token_provider: Callable[[], str] | None
    dynamic_params: bool
    print_verbose: Callable[[str], None]
    timeout: float
    logging_obj: Logging
    optional_params: dict[str, JsonValue]
    litellm_params: dict[str, JsonValue]
    logger_fn: None
    acompletion: bool
    headers: dict[str, str] | None
    client: AzureClient | None


class OSeriesKwargs(TypedDict):
    model_response: ModelResponse
    timeout: float
    optional_params: dict[str, JsonValue]
    litellm_params: dict[str, JsonValue]
    logging_obj: Logging
    model: str | None
    messages: list[Turn] | None
    print_verbose: Callable[[str], None]
    api_key: str | None
    api_base: str
    api_version: str
    dynamic_params: bool | None
    azure_ad_token: str | None
    acompletion: bool
    logger_fn: None
    headers: dict[str, str] | None
    client: AnyClient | None
    organization: str | None
    custom_llm_provider: str | None
    drop_params: bool | None
    shared_session: None


class LiteLlmParamsView(TypedDict, total=False):
    azure_ad_token: JsonValue
    max_retries: JsonValue
    timeout: JsonValue


class PreArgs(BaseModel):
    model_config = ConfigDict(extra="allow")

    headers: dict[str, JsonValue] | None = None
    api_base: JsonValue = None
    api_version: str | None = None
    complete_input_dict: JsonValue = None
    openai_sdk: bool | None = None
    acompletion: bool = False


class PostArgs(BaseModel):
    model_config = ConfigDict(extra="allow")

    headers: dict[str, JsonValue] | None = None
    api_base: JsonValue = None
    api_version: str | None = None
    complete_input_dict: JsonValue = None
    openai_sdk: bool | None = None


class Outcome(BaseModel):
    model_config = ConfigDict(extra="forbid")

    outcome: str
    reply: JsonValue = None
    error: JsonValue = None
    forward: JsonValue = None


class OriginalView(TypedDict, total=False):
    id: str


class MessageView(TypedDict, total=False):
    role: str
    content: str | None
    tool_calls: list[JsonValue] | None


class ChoiceView(TypedDict, total=False):
    index: int
    finish_reason: str | None
    message: MessageView


class UsageView(TypedDict, total=False):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int


class ReplyView(TypedDict, total=False):
    id: str
    model: str
    created: int
    choices: list[ChoiceView]
    usage: UsageView | None


class ErrorView(Protocol):
    status_code: int | None
    message: str
    headers: httpx.Headers | dict[str, str] | None
    body: JsonValue


class HiddenResponse(Protocol):
    _hidden_params: dict[str, JsonValue | httpx.Headers]


class JsonDump(Protocol):
    def model_dump_json(self) -> str: ...


class SyncChat(Protocol):
    def completion(self, **kwargs: Unpack[CompletionKwargs]) -> JsonDump: ...


class AsyncChat(Protocol):
    async def completion(self, **kwargs: Unpack[CompletionKwargs]) -> JsonDump: ...


class OSeriesChat(Protocol):
    def completion(self, **kwargs: Unpack[OSeriesKwargs]) -> JsonDump: ...


JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
BODY: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])
REPLY: Final[TypeAdapter[ReplyView]] = TypeAdapter(ReplyView)
ORIGINAL: Final[TypeAdapter[OriginalView]] = TypeAdapter(OriginalView)
PRE_ARGS: Final[TypeAdapter[PreArgs]] = TypeAdapter(PreArgs)
POST_ARGS: Final[TypeAdapter[PostArgs]] = TypeAdapter(PostArgs)
PARAMS_VIEW: Final[TypeAdapter[LiteLlmParamsView]] = TypeAdapter(LiteLlmParamsView)


def api_base_of(case: CallCase) -> str:
    base = case.get("api_base")
    if base is None:
        return API_BASE
    return base


def api_version_of(case: CallCase) -> str:
    version = case.get("api_version")
    if version is None:
        return API_VERSION
    return version


def timeout_of(case: CallCase) -> float:
    timeout = case.get("timeout")
    if timeout is None:
        return TIMEOUT
    return timeout


def case_answer(case: CallCase) -> Answer:
    answer = case.get("answer")
    if answer is None:
        return {}
    return answer


def optional_params_of(case: CallCase) -> dict[str, JsonValue]:
    params = case.get("optional_params")
    if params is None:
        return {}
    return dict(params)


def litellm_params_of(case: CallCase) -> dict[str, JsonValue]:
    params = case.get("litellm_params")
    merged: dict[str, JsonValue] = {} if params is None else dict(params)
    base = case.get("base_model")
    if base is not None:
        merged["base_model"] = base
    return merged


def caller_of(case: CallCase) -> JsonValue:
    headers = case.get("headers")
    if headers is None:
        return None
    return JSON.validate_python(headers)


def source_key(case: CallCase) -> str | None:
    if "client_api_key" in case:
        return case["client_api_key"]
    return case.get("api_key")


def client_kind(case: CallCase) -> Literal["sync", "async"]:
    requested = case.get("client")
    if requested is not None:
        return requested
    if case.get("acompletion"):
        return "async"
    return "sync"


def client_for(case: CallCase, fixture: "Fixture") -> AzureClient:
    key = source_key(case)
    token = None if key is not None else case.get("azure_ad_token")
    version = api_version_of(case)
    endpoint = api_base_of(case)
    if client_kind(case) == "async":
        return AsyncAzureOpenAI(
            api_key=key,
            azure_ad_token=token,
            api_version=version,
            azure_endpoint=endpoint,
            max_retries=0,
            http_client=httpx.AsyncClient(transport=httpx.MockTransport(fixture.respond)),
        )
    return AzureOpenAI(
        api_key=key,
        azure_ad_token=token,
        api_version=version,
        azure_endpoint=endpoint,
        max_retries=0,
        http_client=httpx.Client(transport=httpx.MockTransport(fixture)),
    )


def supplied_client(case: CallCase, mock: AzureClient) -> AzureClient | None:
    if case.get("supplied") is not False:
        return mock
    return None


def response_for(answer: Answer, request: httpx.Request) -> httpx.Response:
    error = answer.get("error")
    if error == "timeout":
        raise httpx.ReadTimeout("Request timed out.", request=request)
    if error == "other":
        raise httpx.ConnectError("connection refused", request=request)
    status = answer.get("status")
    if status is None:
        status = 400 if error == "bad_request" else 200
    headers = answer.get("headers")
    if headers is None:
        headers = {"content-type": "application/json"}
    body = answer.get("body")
    if body is None:
        body = CHAT_BODY
    return httpx.Response(status, headers=headers, content=body.encode(), request=request)


class Fixture:
    def __init__(self, answer: Answer, caller: JsonValue) -> None:
        self.answer = answer
        self.caller = caller
        self.requests: list[JsonValue] = []
        self.started = asyncio.Event()

    def __call__(self, request: httpx.Request) -> httpx.Response:
        self.requests.append(self.view(request))
        return response_for(self.answer, request)

    async def respond(self, request: httpx.Request) -> httpx.Response:
        self.requests.append(self.view(request))
        self.started.set()
        if self.answer.get("error") == "pending":
            await asyncio.Event().wait()
        return response_for(self.answer, request)

    def view(self, request: httpx.Request) -> JsonValue:
        body = BODY.validate_json(request.content)
        body.pop("extra_headers", None)
        return {"data": body, "caller_headers": self.caller}


def pre_view(params: dict[str, JsonValue]) -> JsonValue:
    additional = PRE_ARGS.validate_python(params.get("additional_args"))
    return {
        "input": params.get("input"),
        "api_key": params.get("api_key"),
        "headers": additional.headers,
        "api_base": additional.api_base,
        "api_version": additional.api_version,
        "acompletion": additional.acompletion,
        "complete_input_dict": additional.complete_input_dict,
        "openai_sdk": additional.openai_sdk,
    }


def original_id(params: dict[str, JsonValue]) -> JsonValue:
    try:
        original = ORIGINAL.validate_python(params.get("original_response"))
    except ValidationError:
        return None
    return original.get("id")


def post_view(params: dict[str, JsonValue]) -> JsonValue:
    additional = POST_ARGS.validate_python(params.get("additional_args"))
    return {
        "input": params.get("input"),
        "headers": additional.headers,
        "api_base": additional.api_base,
        "api_version": additional.api_version,
        "acompletion": False,
        "complete_input_dict": additional.complete_input_dict,
        "openai_sdk": additional.openai_sdk,
        "original_id": original_id(params),
    }


class Recorder(Logging):
    pre: list[JsonValue]
    post: list[JsonValue]

    def __init__(self) -> None:
        super().__init__(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/litellm_logging.py#L576
            model=MODEL,
            messages=None,
            stream=False,
            call_type="completion",
            start_time=datetime(2024, 1, 2, tzinfo=UTC),
            litellm_call_id="call",
            function_id="function",
        )
        self.pre = []
        self.post = []

    @override
    def pre_call(self, *args: JsonValue, **params: JsonValue) -> None:
        del args
        self.pre.append(pre_view(params))

    @override
    def post_call(self, *args: JsonValue, **params: JsonValue) -> None:
        del args
        self.post.append(post_view(params))


def stored_headers(recorder: Recorder) -> JsonValue:
    details: dict[str, JsonValue] = recorder.model_call_details
    headers = details.get("response_headers")
    if headers is None:
        return None
    return BODY.validate_python(headers)


def choice_view(reply: ReplyView) -> JsonValue:
    choices = reply.get("choices")
    if choices is None:
        return None
    if len(choices) == 0:
        return None
    choice = choices[0]
    message = choice.get("message")
    if message is None:
        return None
    return {
        "index": choice.get("index"),
        "finish_reason": choice.get("finish_reason"),
        "role": message.get("role"),
        "content": message.get("content"),
        "has_tool_calls": message.get("tool_calls") is not None,
    }


def usage_view(usage: UsageView | None) -> JsonValue:
    if usage is None:
        return {"prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0}
    return {
        "prompt_tokens": usage.get("prompt_tokens"),
        "completion_tokens": usage.get("completion_tokens"),
        "total_tokens": usage.get("total_tokens"),
    }


def hidden_view(result: JsonDump) -> JsonValue:
    hidden = cast(HiddenResponse, result)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/llm_response_utils/convert_dict_to_response.py#L742-L745
    params = hidden._hidden_params  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/llm_response_utils/convert_dict_to_response.py#L742-L745
    view: dict[str, JsonValue] = {
        "additional_headers": BODY.validate_python(params.get("additional_headers")),
    }
    raw = params.get("headers")
    if raw is not None:
        view["headers"] = BODY.validate_python(raw)
    return view


def reply_view(result: JsonDump) -> JsonValue:
    reply = REPLY.validate_json(result.model_dump_json())
    return {
        "id": reply.get("id"),
        "model": reply.get("model"),
        "created": reply.get("created"),
        "choice": choice_view(reply),
        "usage": usage_view(reply.get("usage")),
        "hidden": hidden_view(result),
    }


def error_fields(
    status: int | None,
    headers: httpx.Headers | dict[str, str] | None,
    message: str,
    has_body: bool,
) -> JsonValue:
    retry: JsonValue = None
    if headers is not None:
        retry = BODY.validate_python(headers).get("retry-after")
    return {
        "status": status,
        "retry_after": retry,
        "has_headers": headers is not None,
        "has_body": has_body,
        "has_timeout_diagnostic": TIMEOUT_DIAGNOSTIC in message,
    }


def azure_error_view(error: AzureOpenAIError) -> JsonValue:
    view = cast(ErrorView, error)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L46-L63
    return error_fields(
        view.status_code, view.headers, view.message, view.body is not None
    )


def sdk_error_view(error: OpenAIError) -> JsonValue:
    match error:
        case APITimeoutError():
            return error_fields(500, None, error.message, False)
        case APIStatusError():
            return error_fields(
                error.status_code, error.response.headers, error.message, error.body is not None
            )
        case APIResponseValidationError():
            return error_fields(
                error.status_code, error.response.headers, error.message, error.body is not None
            )
        case APIError():
            return error_fields(500, None, error.message, error.body is not None)
        case OpenAIError():
            return error_fields(500, None, str(error), False)


def kind_name(client: AnyClient) -> str:
    kind = KINDS.get(type(client).__name__)
    if kind is None:
        return "other"
    return kind


def client_view(client: AnyClient | None) -> JsonValue:
    if client is None:
        return None
    return {"kind": kind_name(client), "api_key": client.api_key}


def params_view(params: JsonValue) -> JsonValue:
    if params is None:
        return None
    view = PARAMS_VIEW.validate_python(params)
    return {
        "azure_ad_token": view.get("azure_ad_token"),
        "max_retries": view.get("max_retries"),
        "timeout": view.get("timeout"),
    }


def forward_view(params: dict[str, JsonValue]) -> JsonValue:
    client = cast(AnyClient | None, params.get("client"))
    return {
        "model": params.get("model"),
        "api_base": params.get("api_base"),
        "api_version": params.get("api_version"),
        "api_key": params.get("api_key"),
        "azure_ad_token": params.get("azure_ad_token"),
        "timeout": params.get("timeout"),
        "acompletion": params.get("acompletion"),
        "organization": params.get("organization"),
        "drop_params": params.get("drop_params"),
        "shared_session": params.get("shared_session"),
        "dynamic_params": params.get("dynamic_params"),
        "headers": params.get("headers"),
        "optional_params": params.get("optional_params"),
        "litellm_params": params_view(params.get("litellm_params")),
        "client": client_view(client),
    }


def completion_kwargs(
    case: CallCase, recorder: Recorder, client: AzureClient | None
) -> CompletionKwargs:
    return {
        "model": case.get("model"),
        "messages": case.get("messages"),
        "model_response": ModelResponse(),
        "api_key": case.get("api_key"),
        "api_base": api_base_of(case),
        "api_version": api_version_of(case),
        "api_type": "azure",
        "azure_ad_token": case.get("azure_ad_token"),
        "azure_ad_token_provider": None,
        "dynamic_params": False,
        "print_verbose": print,
        "timeout": timeout_of(case),
        "logging_obj": recorder,
        "optional_params": optional_params_of(case),
        "litellm_params": litellm_params_of(case),
        "logger_fn": None,
        "acompletion": bool(case.get("acompletion")),
        "headers": case.get("headers"),
        "client": client,
    }


def o_series_kwargs(
    case: CallCase, recorder: Recorder, client: AzureClient | None
) -> OSeriesKwargs:
    return {
        "model_response": ModelResponse(),
        "timeout": timeout_of(case),
        "optional_params": optional_params_of(case),
        "litellm_params": litellm_params_of(case),
        "logging_obj": recorder,
        "model": case.get("model"),
        "messages": case.get("messages"),
        "print_verbose": print,
        "api_key": case.get("api_key"),
        "api_base": api_base_of(case),
        "api_version": api_version_of(case),
        "dynamic_params": None,
        "azure_ad_token": case.get("azure_ad_token"),
        "acompletion": bool(case.get("acompletion")),
        "logger_fn": None,
        "headers": case.get("headers"),
        "client": client,
        "organization": case.get("organization"),
        "custom_llm_provider": None,
        "drop_params": case.get("drop_params"),
        "shared_session": None,
    }


class ScriptedAzure:
    mock: AzureClient
    gets: list[JsonValue]
    gateways: list[JsonValue]

    def get_azure_openai_client(  # noqa: PLR0913  # pyright: ignore[reportIncompatibleMethodOverride]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L480-L489
        self,
        api_key: str | None,
        api_base: str | None,
        api_version: str | None = None,
        client: AnyClient | None = None,
        litellm_params: dict[str, JsonValue] | None = None,
        _is_async: bool = False,
        model: str | None = None,
    ) -> AnyClient | None:
        params: dict[str, JsonValue] = {} if litellm_params is None else litellm_params
        self.gets.append(
            {
                "api_version": api_version,
                "api_base": api_base,
                "api_key": api_key,
                "model": model,
                "is_async": _is_async,
                "supplied": client is not None,
                "litellm_params": {
                    "azure_ad_token": params.get("azure_ad_token"),
                    "max_retries": params.get("max_retries"),
                    "timeout": params.get("timeout"),
                },
            }
        )
        if client is None:
            return self.mock
        return client

    def _init_azure_client_for_cloudflare_ai_gateway(  # noqa: PLR0913  # ast-grep-ignore: no-long-names
        self,
        api_base: str,
        model: str,
        api_version: str,
        max_retries: int,
        timeout: float,
        litellm_params: dict[str, JsonValue],
        api_key: str | None,
        azure_ad_token: str | None,
        azure_ad_token_provider: Callable[[], str] | None,
        acompletion: bool,
        client: AzureClient | None = None,
    ) -> AzureClient:
        del litellm_params, azure_ad_token, azure_ad_token_provider
        if client is None:
            self.gateways.append(
                {
                    "api_base": api_base,
                    "model": model,
                    "api_version": api_version,
                    "max_retries": max_retries,
                    "timeout": timeout,
                    "api_key": api_key,
                    "acompletion": acompletion,
                }
            )
            return self.mock
        return client


class ScriptedChat(ScriptedAzure, AzureChatCompletion):
    def __init__(self, mock: AzureClient) -> None:
        super().__init__()
        self.mock = mock
        self.gets = []
        self.gateways = []


class ScriptedOSeries(ScriptedAzure, AzureOpenAIO1ChatCompletion):
    def __init__(self, mock: AzureClient) -> None:
        super().__init__()
        self.mock = mock
        self.gets = []
        self.gateways = []


class Capture:
    def __init__(self) -> None:
        self.calls: list[JsonValue] = []

    def __call__(self, **params: JsonValue) -> ModelResponse:
        self.calls.append(forward_view(params))
        return ModelResponse()


def report(
    handler: ScriptedAzure, fixture: Fixture, recorder: Recorder, result: Outcome
) -> JsonValue:
    return {
        "outcome": result.outcome,
        "gets": handler.gets,
        "gateways": handler.gateways,
        "requests": fixture.requests,
        "pre": recorder.pre,
        "post": recorder.post,
        "response_headers": stored_headers(recorder),
        "reply": result.reply,
        "error": result.error,
        "forward": result.forward,
    }


async def cancelled_call(
    handler: ScriptedChat, kwargs: CompletionKwargs, fixture: Fixture
) -> None:
    task = asyncio.create_task(cast(AsyncChat, handler).completion(**kwargs))
    await fixture.started.wait()
    task.cancel()
    try:
        await task
    except asyncio.CancelledError:
        return


async def completion_call(case: CallCase) -> JsonValue:
    fixture = Fixture(case_answer(case), caller_of(case))
    mock = client_for(case, fixture)
    handler = ScriptedChat(mock)
    recorder = Recorder()
    kwargs = completion_kwargs(case, recorder, supplied_client(case, mock))
    if case.get("cancel"):
        await cancelled_call(handler, kwargs, fixture)
        return report(handler, fixture, recorder, Outcome(outcome="cancelled"))
    try:
        if kwargs["acompletion"]:
            result = await cast(AsyncChat, handler).completion(**kwargs)
        else:
            result = cast(SyncChat, handler).completion(**kwargs)
    except AzureOpenAIError as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L46
        return report(handler, fixture, recorder, Outcome(outcome="error", error=azure_error_view(error)))
    except OpenAIError as error:
        return report(handler, fixture, recorder, Outcome(outcome="error", error=sdk_error_view(error)))
    return report(handler, fixture, recorder, Outcome(outcome="buffered", reply=reply_view(result)))


async def o_series_call(case: CallCase) -> JsonValue:
    fixture = Fixture(case_answer(case), caller_of(case))
    mock = client_for(case, fixture)
    handler = ScriptedOSeries(mock)
    recorder = Recorder()
    kwargs = o_series_kwargs(case, recorder, supplied_client(case, mock))
    capture = Capture()
    original = OpenAIChatCompletion.completion
    OpenAIChatCompletion.completion = capture  # type: ignore[method-assign, assignment]  # pyright: ignore[reportAttributeAccessIssue]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/openai.py#L645-L646
    try:
        cast(OSeriesChat, handler).completion(**kwargs)
    finally:
        OpenAIChatCompletion.completion = original  # type: ignore[method-assign, assignment]  # pyright: ignore[reportAttributeAccessIssue]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/openai.py#L645-L646
    return report(
        handler,
        fixture,
        recorder,
        Outcome(outcome="delegated", forward=capture.calls[0]),
    )


def run(input_value: Input) -> JsonValue:
    if input_value["function"] == "completion":
        return {"calls": [asyncio.run(completion_call(case)) for case in input_value["cases"]]}
    return {"calls": [asyncio.run(o_series_call(case)) for case in input_value["cases"]]}
