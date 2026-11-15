from collections.abc import Callable, Mapping
from typing import ClassVar, Literal, NotRequired, ParamSpec, Protocol, TypedDict

import httpx
import requests
from litellm.types.utils import EmbeddingResponse
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter
from requests import PreparedRequest
from requests.adapters import HTTPAdapter
from typing_extensions import override

JSON_ADAPTER = TypeAdapter(JsonValue)
FAILURES = ("DatabricksException", "BadRequestError")
EMBEDDING_BODY = {
    "object": "list",
    "data": [{"object": "embedding", "index": 0, "embedding": [0.1]}],
    "model": "databricks-bge-large-en",
    "usage": {"prompt_tokens": 1, "total_tokens": 1},
}
Params = ParamSpec("Params")


class OauthScript(TypedDict, total=False):
    status: int
    body: str
    transport: str


class ValidateCase(TypedDict, total=False):
    api_key: str | None
    api_base: str | None
    endpoint: Literal["chat_completions", "embeddings"]
    custom_endpoint: bool
    headers: dict[str, str] | None
    custom_user_agent: str | None
    oauth: OauthScript


class ApiBaseCase(TypedDict):
    api_base: str | None
    use_ai_gateway: bool


class UserAgentCase(TypedDict, total=False):
    custom: str | None


class EmbeddingCase(TypedDict):
    model: str
    input: JsonValue
    optional_params: NotRequired[dict[str, JsonValue]]
    api_key: NotRequired[str | None]
    api_base: NotRequired[str | None]
    custom_endpoint: NotRequired[bool]
    headers: NotRequired[dict[str, str] | None]


class UrlCase(TypedDict, total=False):
    api_base: str | None


class CostCase(TypedDict):
    models: list[str]
    service_tier: NotRequired[str | None]


class Input(TypedDict, total=False):
    function: Literal[
        "validate",
        "api_base",
        "user_agent",
        "redaction",
        "cost",
        "endpoints",
    ]
    cases: list[JsonValue]
    headers: dict[str, str]
    data: JsonValue
    embedding: list[JsonValue]
    responses: list[JsonValue]


class Failure(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    status_code: int


class Logging:
    def __init__(self) -> None:
        self.entries: list[dict[str, JsonValue]] = []

    def pre_call(self, **params: JsonValue) -> None:
        entry = dict(params)
        entry["phase"] = "pre"
        self.entries.append(entry)

    def post_call(self, **params: JsonValue) -> None:
        entry = dict(params)
        entry["phase"] = "post"
        self.entries.append(entry)


# LiteLLM's Databricks members use bare dict and list, so these protocols restate the types
Endpoint = Literal["chat_completions", "embeddings"]
Headers = dict[str, str]
AuthCall = Callable[
    [str | None, str | None, Endpoint, bool | None, Headers | None, str | None],
    tuple[str, Headers],
]
EmbedCall = Callable[
    [
        str,
        list[JsonValue],
        float,
        Logging,
        str | None,
        str | None,
        dict[str, JsonValue],
        EmbeddingResponse | None,
        httpx.Client,
        bool,
        bool | None,
        Headers | None,
    ],
    EmbeddingResponse,
]


class AuthApi(Protocol):
    @property
    def databricks_validate_environment(self) -> AuthCall: ...


class ChatApi(Protocol):
    def get_complete_url(
        self,
        api_base: str | None,
        api_key: str | None,
        model: str,
        optional_params: dict[str, str],
        litellm_params: dict[str, str],
    ) -> str: ...


class ResponsesApi(Protocol):
    def get_complete_url(self, api_base: str | None, litellm_params: dict[str, str]) -> str: ...


class EmbeddingApi(Protocol):
    @property
    def embedding(self) -> EmbedCall: ...


class FakeResponse(requests.Response):
    def __init__(self, script: OauthScript) -> None:
        super().__init__()
        status = script.get("status")
        self.status_code = 200 if status is None else status
        body = script.get("body")
        self.body = "" if body is None else body

    @property
    @override
    def text(self) -> str:
        return self.body


class RecordingAdapter(HTTPAdapter):
    script: ClassVar[OauthScript] = {}
    sent: ClassVar[dict[str, JsonValue] | None] = None

    @override
    def send(
        self,
        request: PreparedRequest,
        stream: bool = False,
        timeout: None | float | tuple[float, float] | tuple[float, None] = None,
        verify: bool | str = True,
        cert: None | bytes | str | tuple[bytes | str, bytes | str] = None,
        proxies: Mapping[str, str] | None = None,
    ) -> requests.Response:
        RecordingAdapter.sent = {
            "url": TypeAdapter(str).validate_python(request.url),
            "body": TypeAdapter(str).validate_python(request.body),
            "authorization": TypeAdapter(str).validate_python(request.headers["Authorization"]),
            "content_type": TypeAdapter(str).validate_python(request.headers["Content-Type"]),
            "timeout_seconds": int(TypeAdapter(float).validate_python(timeout)),
        }
        transport = self.script.get("transport")
        if transport is not None:
            raise requests.RequestException(transport)
        return FakeResponse(self.script)


class EmbeddingTransport:
    def __init__(self) -> None:
        self.url: str | None = None
        self.headers: dict[str, str] = {}
        self.body: dict[str, JsonValue] = {}

    def handle(self, request: httpx.Request) -> httpx.Response:
        self.url = str(request.url)
        self.headers = dict(request.headers)
        self.body = TypeAdapter(dict[str, JsonValue]).validate_json(request.content)
        return httpx.Response(200, json=EMBEDDING_BODY)

    def output(self, watched: list[str]) -> dict[str, JsonValue]:
        headers: dict[str, JsonValue] = {
            name: value for name, value in self.headers.items() if name in watched
        }
        params: dict[str, JsonValue] = {
            name: value for name, value in self.body.items() if name not in ("model", "input")
        }
        output: dict[str, JsonValue] = {
            "url": self.url,
            "headers": headers,
            "optional_params": params,
        }
        return output


def cases_of(input_value: Input) -> list[JsonValue]:
    cases = input_value.get("cases")
    if cases is None:
        raise ValueError("cases is required")
    return cases


def guarded(
    call: Callable[Params, dict[str, JsonValue]],
    *args: Params.args,
    **kwargs: Params.kwargs,
) -> dict[str, JsonValue]:
    try:
        return call(*args, **kwargs)
    except Exception as error:
        if type(error).__name__ not in FAILURES:
            raise
        failure = Failure.model_validate(error)
        return {"status": failure.status_code}
