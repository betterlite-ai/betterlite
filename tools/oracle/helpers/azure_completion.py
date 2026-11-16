import asyncio
from datetime import UTC, datetime
from typing import Final, Literal, Protocol, TypedDict, cast

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.azure.common_utils import AzureOpenAIError
from litellm.llms.azure.completion.handler import AzureTextCompletion
from litellm.llms.azure.completion.transformation import AzureOpenAITextConfig
from litellm.types.utils import ModelResponse
from openai import AsyncAzureOpenAI, AzureOpenAI
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter
from typing_extensions import override

litellm.suppress_debug_info = True

MODEL: Final = "gpt-35-turbo-instruct"
API_BASE: Final = "https://unit.test"
API_VERSION: Final = "2024-02-01"
CREATED_FLOOR: Final = 1_000_000_000
BODY: Final = (
    '{"id":"cmpl-1","object":"text_completion","created":1700000000,'
    '"model":"gpt-35-turbo-instruct",'
    '"choices":[{"text":"Hello!","index":0,"logprobs":null,"finish_reason":"stop"}],'
    '"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}}'
)
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)


class Turn(TypedDict):
    role: str
    content: str | list[dict[str, JsonValue]]


class Answer(TypedDict, total=False):
    status: int
    headers: dict[str, str]
    body: str
    events: list[str]


class CallCase(TypedDict, total=False):
    model: str | None
    messages: list[Turn] | None
    api_key: str | None
    client_api_key: str | None
    azure_ad_token: str | None
    api_base: str | None
    api_version: str | None
    timeout: float | None
    optional_params: dict[str, JsonValue]
    litellm_params: dict[str, object]
    acompletion: bool
    headers: dict[str, str] | None
    data: dict[str, JsonValue]
    answer: Answer


class CompletionInput(TypedDict):
    function: Literal["completion"]
    cases: list[CallCase]


class StreamInput(TypedDict):
    function: Literal["stream"]
    cases: list[CallCase]


class AuthCase(TypedDict):
    api_key: str | None
    azure_ad_token: str | None


class AuthInput(TypedDict):
    function: Literal["auth"]
    cases: list[AuthCase]


class ConfigConstructor(TypedDict, total=False):
    frequency_penalty: int | None
    logit_bias: dict[str, int] | None
    max_tokens: int | None
    n: int | None
    presence_penalty: int | None
    stop: str | list[str] | None
    temperature: int | None
    top_p: int | None


class ConfigInput(TypedDict):
    function: Literal["config"]
    constructor: ConfigConstructor


Input = CompletionInput | StreamInput | AuthInput | ConfigInput


class PreArgs(BaseModel):
    model_config = ConfigDict(extra="allow")

    headers: dict[str, JsonValue] | None = None
    api_version: str | None = None
    api_base: object = None
    complete_input_dict: dict[str, JsonValue] | None = None
    acompletion: bool = False


class PostArgs(BaseModel):
    model_config = ConfigDict(extra="allow")

    headers: dict[str, str] | None = None
    api_version: str | None = None
    api_base: str | None = None


class OriginalView(TypedDict):
    id: str


class ErrorView(Protocol):
    status_code: int | None
    message: str
    headers: httpx.Headers | dict[str, str] | None


class JsonDump(Protocol):
    def model_dump_json(self) -> str: ...


class SyncStream(Protocol):
    def __next__(self) -> JsonDump: ...


class AsyncStream(Protocol):
    async def __anext__(self) -> JsonDump: ...


class MessageView(TypedDict):
    content: str | None
    role: str
    tool_calls: JsonValue | None
    function_call: JsonValue | None


class ReplyChoiceView(TypedDict):
    index: int
    finish_reason: str | None
    message: MessageView


class DetailsView(TypedDict, total=False):
    reasoning_tokens: JsonValue


class UsageView(TypedDict, total=False):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int
    reasoning_tokens: JsonValue
    queue_time: JsonValue
    completion_tokens_details: DetailsView | None


class ReplyView(TypedDict):
    id: str
    model: str
    created: int
    choices: list[ReplyChoiceView]
    usage: UsageView | None


class DeltaView(TypedDict, total=False):
    content: str | None
    role: str | None


class ChunkChoiceView(TypedDict):
    index: int
    delta: DeltaView
    finish_reason: str | None


class ChunkView(TypedDict):
    model: str
    created: int
    choices: list[ChunkChoiceView]


REPLY: Final[TypeAdapter[ReplyView]] = TypeAdapter(ReplyView)
CHUNK: Final[TypeAdapter[ChunkView]] = TypeAdapter(ChunkView)
ORIGINAL: Final[TypeAdapter[OriginalView]] = TypeAdapter(OriginalView)
PRE_ARGS: Final[TypeAdapter[PreArgs]] = TypeAdapter(PreArgs)
POST_ARGS: Final[TypeAdapter[PostArgs]] = TypeAdapter(PostArgs)


class Recorder(Logging):
    def __init__(self) -> None:
        super().__init__(
            model=MODEL,
            messages=None,
            stream=False,
            call_type="completion",
            start_time=datetime(2024, 1, 2, tzinfo=UTC),
            litellm_call_id="call",
            function_id="function",
        )
        self.pre: list[JsonValue] = []
        self.post: list[JsonValue] = []

    @override
    def pre_call(self, *args: JsonValue, **params: JsonValue) -> None:
        del args
        additional = PRE_ARGS.validate_python(params.get("additional_args"))
        self.pre.append(
            {
                "input": params.get("input"),
                "headers": additional.headers,
                "api_base": pre_api_base(additional),
                "api_version": None if additional.acompletion else additional.api_version,
                "acompletion": additional.acompletion,
                "complete_input_dict": additional.complete_input_dict,
            }
        )

    @override
    def post_call(self, *args: JsonValue, **params: JsonValue) -> None:
        del args
        additional = POST_ARGS.validate_python(params.get("additional_args"))
        original = params.get("original_response")
        self.post.append(
            {
                "input": params.get("input"),
                "headers": additional.headers,
                "api_base": additional.api_base,
                "api_version": additional.api_version,
                "original_id": ORIGINAL.validate_python(original)["id"],
            }
        )


class Fixture:
    def __init__(self, answer: Answer) -> None:
        self.answer = answer
        self.requests: list[JsonValue] = []

    def __call__(self, request: httpx.Request) -> httpx.Response:
        self.requests.append({"data": JSON.validate_json(request.content)})
        return response_for(self.answer, request)


def response_for(answer: Answer, request: httpx.Request) -> httpx.Response:
    status = answer.get("status", 200)
    headers = answer.get("headers", {"content-type": "application/json"})
    events = answer.get("events")
    if events is not None:
        return httpx.Response(
            status, headers=headers, content="".join(events).encode(), request=request
        )
    return httpx.Response(
        status, headers=headers, content=answer.get("body", BODY).encode(), request=request
    )


def pre_api_base(additional: PreArgs) -> JsonValue:
    if additional.acompletion:
        return None
    return JSON.validate_python(additional.api_base)


def source_key(case: CallCase) -> str | None:
    if "client_api_key" in case:
        return case["client_api_key"]
    return case.get("api_key")


def sync_client(case: CallCase, fixture: Fixture) -> AzureOpenAI:
    key = source_key(case)
    token = None if key is not None else case.get("azure_ad_token")
    return AzureOpenAI(
        api_key=key,
        azure_ad_token=token,
        api_version=case.get("api_version", API_VERSION),
        azure_endpoint=case.get("api_base", API_BASE),
        max_retries=0,
        http_client=httpx.Client(transport=httpx.MockTransport(fixture)),
    )


def async_client(case: CallCase, fixture: Fixture) -> AsyncAzureOpenAI:
    key = source_key(case)
    token = None if key is not None else case.get("azure_ad_token")
    return AsyncAzureOpenAI(
        api_key=key,
        azure_ad_token=token,
        api_version=case.get("api_version", API_VERSION),
        azure_endpoint=case.get("api_base", API_BASE),
        max_retries=0,
        http_client=httpx.AsyncClient(transport=httpx.MockTransport(fixture)),
    )


def client_for(case: CallCase, fixture: Fixture) -> AzureOpenAI | AsyncAzureOpenAI:
    requested = case.get("client", "async" if case.get("acompletion", False) else "sync")
    if requested == "async":
        return async_client(case, fixture)
    return sync_client(case, fixture)


def report(  # noqa: PLR0913 https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/completion/handler.py#L31-L186
    fixture: Fixture,
    recorder: Recorder,
    outcome: str,
    reply: JsonValue,
    chunks: JsonValue,
    error: JsonValue,
) -> JsonValue:
    return {
        "outcome": outcome,
        "requests": fixture.requests,
        "pre": recorder.pre,
        "post": recorder.post,
        "reply": reply,
        "chunks": chunks,
        "error": error,
    }


def error_view(error: object) -> JsonValue:
    view = cast(ErrorView, error)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L46-L63
    return {
        "status": view.status_code,
        "message": view.message,
        "retry_after": header_value(view.headers, "retry-after"),
    }


def header_value(headers: httpx.Headers | dict[str, str] | None, name: str) -> str | None:
    if headers is None:
        return None
    return headers.get(name)


def detail_value(usage: UsageView) -> JsonValue:
    details = usage.get("completion_tokens_details")
    if details is None:
        return None
    return details.get("reasoning_tokens")


def usage_reasoning(usage: UsageView) -> JsonValue:
    direct = usage.get("reasoning_tokens")
    if direct is not None:
        return direct
    return detail_value(usage)


def usage_view(usage: UsageView | None) -> JsonValue:
    if usage is None:
        return None
    return {
        "prompt_tokens": usage.get("prompt_tokens", 0),
        "completion_tokens": usage.get("completion_tokens", 0),
        "total_tokens": usage.get("total_tokens", 0),
        "reasoning_tokens": usage_reasoning(usage),
        "queue_time": usage.get("queue_time"),
        "details": usage.get("completion_tokens_details") is not None,
    }


def choice_view(choice: ReplyChoiceView) -> JsonValue:
    return {
        "index": choice["index"],
        "finish_reason": choice["finish_reason"],
        "content": choice["message"].get("content"),
        "role": choice["message"]["role"],
        "tool_calls": choice["message"].get("tool_calls"),
        "function_call": choice["message"].get("function_call"),
    }


def reply_view(view: ReplyView) -> JsonValue:
    return {
        "id": view["id"],
        "model": view["model"],
        "created_is_current": view["created"] > CREATED_FLOOR,
        "choices": [choice_view(choice) for choice in view["choices"]],
        "usage": usage_view(view["usage"]),
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


def sync_chunk(stream: SyncStream) -> JsonValue:
    try:
        return chunk_view(CHUNK.validate_json(stream.__next__().model_dump_json()))
    except StopIteration:
        return None


async def next_chunk(stream: AsyncStream) -> JsonValue:
    try:
        chunk = await stream.__anext__()
    except StopAsyncIteration:
        return None
    return chunk_view(CHUNK.validate_json(chunk.model_dump_json()))


def sync_chunks(stream: SyncStream) -> list[JsonValue]:
    chunks: list[JsonValue] = []
    chunk = sync_chunk(stream)
    while chunk is not None:
        chunks.append(chunk)
        chunk = sync_chunk(stream)
    return chunks


async def async_chunks(stream: AsyncStream) -> list[JsonValue]:
    chunks: list[JsonValue] = []
    chunk = await next_chunk(stream)
    while chunk is not None:
        chunks.append(chunk)
        chunk = await next_chunk(stream)
    return chunks


def sync_completion(case: CallCase) -> JsonValue:
    fixture = Fixture(case.get("answer", {}))
    recorder = Recorder()
    client = client_for(case, fixture)
    try:
        result = AzureTextCompletion().completion(
            model=cast(str, case.get("model")),
            messages=cast(list[Turn], case.get("messages")),
            model_response=ModelResponse(),
            api_key=case.get("api_key"),
            api_base=cast(str, case.get("api_base")),
            api_version=cast(str, case.get("api_version", API_VERSION)),
            api_type="azure",
            azure_ad_token=case.get("azure_ad_token"),
            azure_ad_token_provider=None,
            print_verbose=print,
            timeout=case.get("timeout"),
            logging_obj=recorder,
            optional_params=dict(case.get("optional_params", {})),
            litellm_params=case.get("litellm_params", {}),
            logger_fn=None,
            acompletion=False,
            headers=case.get("headers"),
            client=client,
        )
    except AzureOpenAIError as error:
        return report(fixture, recorder, "error", None, None, error_view(error))
    if case.get("optional_params", {}).get("stream") is True:
        chunks = sync_chunks(cast(SyncStream, result))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/completion/handler.py#L88-L100
        return report(fixture, recorder, "stream", None, chunks, None)
    reply = REPLY.validate_json(cast(JsonDump, result).model_dump_json())
    return report(fixture, recorder, "buffered", reply_view(reply), None, None)


async def async_completion(case: CallCase) -> JsonValue:
    fixture = Fixture(case.get("answer", {}))
    recorder = Recorder()
    client = client_for(case, fixture)
    streamed = bool(case.get("optional_params", {}).get("stream", False))
    try:
        result = await AzureTextCompletion().completion(
            model=cast(str, case.get("model")),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/completion/handler.py#L188-L247
            messages=cast(list[Turn], case.get("messages")),
            model_response=ModelResponse(),
            api_key=case.get("api_key"),
            api_base=cast(str, case.get("api_base")),
            api_version=cast(str, case.get("api_version", API_VERSION)),
            api_type="azure",
            azure_ad_token=case.get("azure_ad_token"),
            azure_ad_token_provider=None,
            print_verbose=print,
            timeout=case.get("timeout"),
            logging_obj=recorder,
            optional_params=dict(case.get("optional_params", {})),
            litellm_params=case.get("litellm_params", {}),
            logger_fn=None,
            acompletion=True,
            headers=case.get("headers"),
            client=client,
        )
    except AzureOpenAIError as error:
        return report(fixture, recorder, "error", None, None, error_view(error))
    if streamed:
        chunks = await async_chunks(cast(AsyncStream, result))
        return report(fixture, recorder, "stream", None, chunks, None)
    reply = REPLY.validate_json(cast(JsonDump, result).model_dump_json())
    return report(fixture, recorder, "buffered", reply_view(reply), None, None)


def sync_stream_call(case: CallCase) -> JsonValue:
    fixture = Fixture(case.get("answer", {}))
    recorder = Recorder()
    client = client_for(case, fixture)
    try:
        stream = AzureTextCompletion().streaming(
            logging_obj=recorder,
            api_base=cast(str, case.get("api_base")),
            api_key=case.get("api_key"),
            api_version=cast(str, case.get("api_version", API_VERSION)),
            data=dict(case.get("data", {})),
            model=cast(str, case.get("model")),
            timeout=case.get("timeout"),
            azure_ad_token=case.get("azure_ad_token"),
            client=client,
            litellm_params={},
        )
    except AzureOpenAIError as error:
        return report(fixture, recorder, "error", None, None, error_view(error))
    chunks = sync_chunks(cast(SyncStream, stream))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/completion/handler.py#L249-L300
    return report(fixture, recorder, "stream", None, chunks, None)


async def async_stream_call(case: CallCase) -> JsonValue:
    fixture = Fixture(case.get("answer", {}))
    recorder = Recorder()
    client = client_for(case, fixture)
    try:
        stream = await AzureTextCompletion().async_streaming(
            logging_obj=recorder,
            api_base=cast(str, case.get("api_base")),
            api_key=case.get("api_key"),
            api_version=cast(str, case.get("api_version", API_VERSION)),
            data=dict(case.get("data", {})),
            model=cast(str, case.get("model")),
            timeout=case.get("timeout"),
            azure_ad_token=case.get("azure_ad_token"),
            client=client,
            litellm_params={},
        )
    except AzureOpenAIError as error:
        return report(fixture, recorder, "error", None, None, error_view(error))
    chunks = await async_chunks(cast(AsyncStream, stream))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/completion/handler.py#L302-L358
    return report(fixture, recorder, "stream", None, chunks, None)


def completion_call(case: CallCase) -> JsonValue:
    if case.get("acompletion", False):
        return asyncio.run(async_completion(case))
    return sync_completion(case)


def stream_call(case: CallCase) -> JsonValue:
    if case.get("acompletion", False):
        return asyncio.run(async_stream_call(case))
    return sync_stream_call(case)


def auth_case(case: AuthCase) -> JsonValue:
    headers = AzureTextCompletion().validate_environment(
        api_key=case["api_key"], azure_ad_token=case["azure_ad_token"]
    )
    return {"headers": JSON.validate_python(headers)}


def config_fields(config: AzureOpenAITextConfig) -> JsonValue:
    return JSON.validate_python(
        {
            "frequency_penalty": config.frequency_penalty,
            "logit_bias": config.logit_bias,
            "max_tokens": config.max_tokens,
            "n": config.n,
            "presence_penalty": config.presence_penalty,
            "stop": config.stop,
            "temperature": config.temperature,
            "top_p": config.top_p,
        }
    )


def run(input_value: Input) -> JsonValue:
    if input_value["function"] == "completion":
        return {"calls": [completion_call(case) for case in input_value["cases"]]}
    if input_value["function"] == "stream":
        return {"calls": [stream_call(case) for case in input_value["cases"]]}
    if input_value["function"] == "auth":
        return {"results": [auth_case(case) for case in input_value["cases"]]}
    config = AzureOpenAITextConfig(**input_value["constructor"])
    return {"fields": config_fields(config)}
