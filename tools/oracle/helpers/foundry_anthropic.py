from collections.abc import Callable
from typing import Final, Protocol, TypedDict, Unpack, override, runtime_checkable

import httpx
import litellm
from litellm.litellm_core_utils.streaming_handler import CustomStreamWrapper
from litellm.llms.azure_ai.anthropic.handler import AzureAnthropicChatCompletion
from litellm.llms.custom_httpx.http_handler import HTTPHandler
from litellm.types.utils import ModelResponse
from pydantic import ConfigDict, JsonValue, TypeAdapter

litellm.suppress_debug_info = True
TIMEOUT: Final[float] = 1.5
JsonObject = dict[str, JsonValue]
ChatResult = ModelResponse | CustomStreamWrapper
HEADERS: Final[tuple[str, ...]] = (
    "api-key",
    "x-api-key",
    "authorization",
    "anthropic-version",
    "anthropic-beta",
)
BODY_KEYS: Final[tuple[str, ...]] = (
    "max_tokens",
    "temperature",
    "top_p",
    "thinking",
    "extra_body",
    "max_retries",
    "stream_options",
    "stream",
    "json_mode",
    "is_vertex_request",
)


class Input(TypedDict):
    api_base: str
    model: str
    messages: list[JsonObject]
    optional_params: JsonObject
    litellm_params: JsonObject
    caller_headers: dict[str, str]
    api_key: str | None
    azure_ad_token: str | None
    upstream_status: int | None
    upstream_headers: dict[str, str]
    upstream_body: str | None
    upstream_lines: list[str]


class CompletionArguments(TypedDict):
    model: str
    messages: list[JsonObject]
    api_base: str
    custom_llm_provider: str
    custom_prompt_dict: JsonObject
    model_response: ModelResponse
    print_verbose: Callable[[str], None]
    encoding: None
    api_key: str | None
    logging_obj: "Recorder"
    optional_params: JsonObject
    timeout: float
    litellm_params: JsonObject
    acompletion: bool
    logger_fn: None
    headers: dict[str, str]
    client: HTTPHandler


# the part of LiteLLM's Logging object the handler and the stream wrapper reach into
class Logging(Protocol):
    model_call_details: JsonObject

    def pre_call(self, **kwargs: JsonValue) -> None: ...

    def post_call(self, **kwargs: JsonValue) -> None: ...


class Recorder(Logging):
    def __init__(self) -> None:
        self.model_call_details: JsonObject = {}

    @override
    def pre_call(self, **kwargs: JsonValue) -> None:
        del kwargs

    @override
    def post_call(self, **kwargs: JsonValue) -> None:
        del kwargs


class Silent:
    def __call__(self, text: str) -> None:
        del text


@runtime_checkable
class Handler(Protocol):
    def completion(self, **kwargs: Unpack[CompletionArguments]) -> ChatResult: ...


HANDLER: Final[Handler] = TypeAdapter(
    Handler, config=ConfigDict(arbitrary_types_allowed=True)
).validate_python(AzureAnthropicChatCompletion())
PRINT: Final[Silent] = Silent()
POSTED: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])


# LiteLLM joins derived betas out of a set, so the order says nothing about the provider
def header_value(posted: httpx.Headers, name: str) -> str | None:
    if name not in posted:
        return None
    value = posted[name]
    if name != "anthropic-beta":
        return value
    return ",".join(sorted(value.split(",")))


class Endpoint:
    def __init__(
        self, status: int | None, headers: dict[str, str], body: str | None
    ) -> None:
        self.status = status
        self.headers = headers
        self.content = b"" if body is None else body.encode()
        self.requests: list[httpx.Request] = []

    def answer(self, request: httpx.Request) -> httpx.Response:
        self.requests.append(request)
        return httpx.Response(
            200 if self.status is None else self.status,
            headers=self.headers,
            stream=httpx.ByteStream(self.content),
        )


def run(input_value: Input) -> JsonValue:
    endpoint = Endpoint(
        input_value["upstream_status"],
        input_value["upstream_headers"],
        input_value["upstream_body"],
    )
    client = HTTPHandler(
        client=httpx.Client(transport=httpx.MockTransport(endpoint.answer))
    )
    params = dict(input_value["litellm_params"])
    if input_value["azure_ad_token"] is not None:
        params["azure_ad_token"] = input_value["azure_ad_token"]
    HANDLER.completion(
        model=input_value["model"],
        messages=input_value["messages"],
        api_base=input_value["api_base"],
        custom_llm_provider="azure_ai",
        custom_prompt_dict={},
        model_response=ModelResponse(),
        print_verbose=PRINT,
        encoding=None,
        api_key=input_value["api_key"],
        logging_obj=Recorder(),
        optional_params=dict(input_value["optional_params"]),
        timeout=TIMEOUT,
        litellm_params=params,
        acompletion=False,
        logger_fn=None,
        headers=dict(input_value["caller_headers"]),
        client=client,
    )
    answer: JsonObject = {"url": None, "headers": None, "body": None}
    if endpoint.requests:
        request = endpoint.requests[0]
        answer["url"] = str(request.url)
        answer["headers"] = {
            name: header_value(request.headers, name) for name in HEADERS
        }
        fields = POSTED.validate_json(request.content)
        answer["body"] = {
            name: fields.get(name, None) for name in BODY_KEYS
        }
    return answer
