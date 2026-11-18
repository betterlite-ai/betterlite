from collections.abc import Iterator, Mapping
from typing import Final, Literal, Protocol, TypedDict, Unpack, runtime_checkable

import httpx
from litellm.litellm_core_utils.streaming_handler import CustomStreamWrapper
from litellm.llms.bedrock.chat.invoke_transformations.base_invoke_transformation import (
    AmazonInvokeConfig,
)
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler, HTTPHandler
from litellm.types.litellm_params import ControlOptions
from pydantic import ConfigDict, JsonValue, TypeAdapter

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
Wrapper = Literal["async", "sync"]
Client = Literal["none", "async", "sync"]
ClientLabel = Literal["none", "supplied", "module"]
Scenario = Literal["transport_arguments", "wrapper_identity"]
Messages = list[dict[str, JsonValue]]
Payload = dict[str, JsonValue]


class Run(TypedDict):
    wrapper: Wrapper
    client: Client
    model: str
    api_base: str
    headers: dict[str, str]
    data: Payload
    messages: Messages
    json_mode: bool | None
    signed_json_body: str | None
    stream_chunk_size: int | None
    response_headers: dict[str, str]


class Input(TypedDict):
    scenario: Scenario
    runs: list[Run]


class PostView(TypedDict):
    url: str
    body: list[int]


class LogView(TypedDict):
    input: JsonValue
    original_response: JsonValue
    complete_input_dict: JsonValue


class LogArguments(TypedDict):
    input: Messages
    api_key: str
    original_response: str
    additional_args: dict[str, JsonValue]


# The logging object B004 reads at construction and B691 posts to; no other part of Logging.
class LoggingRecorder:
    def __init__(self) -> None:
        self.model_call_details: dict[str, JsonValue] = {"litellm_params": {}}
        self.messages: Messages | None = None
        self.stream_options: dict[str, JsonValue] | None = None
        self.calls: list[LogView] = []

    def post_call(self, **params: Unpack[LogArguments]) -> None:
        self.calls.append(
            {
                "input": JSON.validate_python(params["input"]),
                "original_response": params["original_response"],
                "complete_input_dict": params["additional_args"]["complete_input_dict"],
            }
        )


# What make_call or make_sync_call received, in the order the wrapper's own call site names it.
class SeamCall(TypedDict):
    model: str
    api_base: str
    headers: list[list[str]]
    data: str
    signed_json_body: list[int] | None
    messages: Messages
    fake_stream: bool
    json_mode: bool | None
    provider: str | None
    sync: bool
    stream_chunk_size: int | None


# The wrapper's keyword arguments to make_call, before signature differences between the two calls.
class CommonArguments(TypedDict):
    client: HTTPHandler | AsyncHTTPHandler | None
    api_base: str
    headers: dict[str, str]
    data: str
    model: str
    messages: Messages
    logging_obj: LoggingRecorder
    fake_stream: bool
    json_mode: bool | None
    bedrock_invoke_provider: str | None
    stream_chunk_size: int | None


class SyncArguments(CommonArguments):
    signed_json_body: bytes | None


class EmptyStream:
    def __iter__(self) -> Iterator[JsonValue]:
        return iter([])


# The client a real wrapper call posts through: a real httpx transport that answers from the case.
class MockReply:
    def __init__(self, headers: dict[str, str]) -> None:
        self.headers = headers
        self.posts: list[PostView] = []

    def __call__(self, request: httpx.Request) -> httpx.Response:
        self.posts.append({"url": str(request.url), "body": list(request.content)})
        return httpx.Response(200, headers=self.headers, stream=httpx.ByteStream(b""))


class Recorder:
    def __init__(self) -> None:
        self.calls: list[SeamCall] = []
        self.client: ClientLabel = "none"


class AsyncRecorder(Recorder):
    async def __call__(
        self, **params: Unpack[CommonArguments]
    ) -> tuple[EmptyStream, httpx.Headers]:
        self.client = "none" if params["client"] is None else "supplied"
        self.calls.append(seam_call(params, None, sync=False))
        return EmptyStream(), httpx.Headers()


class SyncRecorder(Recorder):
    def __init__(self, supplied: HTTPHandler | AsyncHTTPHandler | None) -> None:
        super().__init__()
        self.supplied = supplied

    def __call__(self, **params: Unpack[SyncArguments]) -> tuple[EmptyStream, httpx.Headers]:
        kept = self.supplied is not None and params["client"] is self.supplied
        self.client = "supplied" if kept else "module"
        self.calls.append(seam_call(params, params["signed_json_body"], sync=True))
        return EmptyStream(), httpx.Headers()


def seam_call(params: CommonArguments, signed_json_body: bytes | None, *, sync: bool) -> SeamCall:
    return {
        "model": params["model"],
        "api_base": params["api_base"],
        "headers": sorted([name, value] for name, value in params["headers"].items()),
        "data": params["data"],
        "signed_json_body": None if signed_json_body is None else list(signed_json_body),
        "messages": params["messages"],
        "fake_stream": params["fake_stream"],
        "json_mode": params["json_mode"],
        "provider": params["bedrock_invoke_provider"],
        "sync": sync,
        "stream_chunk_size": params["stream_chunk_size"],
    }


@runtime_checkable
class WrapperMethods(Protocol):
    # ast-grep-ignore: no-long-names
    async def get_async_custom_stream_wrapper(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/invoke_transformations/base_invoke_transformation.py#L443-L480
        self,
        model: str,
        custom_llm_provider: str,
        logging_obj: LoggingRecorder,
        api_base: str,
        headers: dict[str, str],
        data: Payload,
        messages: Messages,
        client: AsyncHTTPHandler | None = None,
        signed_json_body: bytes | None = None,
        *,
        json_mode: bool | None = None,
        litellm_params: Mapping[str, ControlOptions],
    ) -> CustomStreamWrapper: ...

    def get_sync_custom_stream_wrapper(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/invoke_transformations/base_invoke_transformation.py#L482-L523
        self,
        model: str,
        custom_llm_provider: str,
        logging_obj: LoggingRecorder,
        api_base: str,
        headers: dict[str, str],
        data: Payload,
        messages: Messages,
        client: HTTPHandler | AsyncHTTPHandler | None = None,
        signed_json_body: bytes | None = None,
        *,
        json_mode: bool | None = None,
        litellm_params: Mapping[str, ControlOptions],
    ) -> CustomStreamWrapper: ...


WRAPPERS: Final[WrapperMethods] = TypeAdapter(
    WrapperMethods, config=ConfigDict(arbitrary_types_allowed=True)
).validate_python(AmazonInvokeConfig())  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/invoke_transformations/base_invoke_transformation.py#L78-L80


def control_params(stream_chunk_size: int | None) -> dict[str, ControlOptions]:
    control = ControlOptions(stream_chunk_size=stream_chunk_size)
    return {"_litellm_control": control}


def signed_body(run: Run) -> bytes | None:
    body = run["signed_json_body"]
    return None if body is None else body.encode()


def async_transport_client(run: Run) -> AsyncHTTPHandler | None:
    if run["client"] == "none":
        return None
    if run["client"] == "async":
        return AsyncHTTPHandler(transport=httpx.MockTransport(MockReply(run["response_headers"])))
    raise ValueError("the async wrapper takes an async client or none")


def sync_transport_client(run: Run) -> HTTPHandler | AsyncHTTPHandler | None:
    if run["client"] == "none":
        return None
    if run["client"] == "async":
        return AsyncHTTPHandler(transport=httpx.MockTransport(MockReply(run["response_headers"])))
    reply = MockReply(run["response_headers"])
    return HTTPHandler(client=httpx.Client(transport=httpx.MockTransport(reply)))


def async_identity_client(run: Run, reply: MockReply) -> AsyncHTTPHandler:
    if run["client"] != "async":
        raise ValueError("a wrapper identity run needs a client of the wrapper's kind")
    return AsyncHTTPHandler(transport=httpx.MockTransport(reply))


def sync_identity_client(run: Run, reply: MockReply) -> HTTPHandler:
    if run["client"] != "sync":
        raise ValueError("a wrapper identity run needs a client of the wrapper's kind")
    return HTTPHandler(client=httpx.Client(transport=httpx.MockTransport(reply)))
