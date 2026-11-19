import httpx
from litellm.llms.azure.azure import AzureChatCompletion
from openai import AsyncAzureOpenAI, AsyncOpenAI, AzureOpenAI, OpenAI
from pydantic import JsonValue
from typing_extensions import override

from azure_stream_errors_records import (
    API_BASE,
    JSON,
    OPTIONAL_HEADER_MAP,
    STATUS,
    TEXT,
    ErrorSpec,
    ExchangeSpec,
    SendSpec,
    StreamCase,
)


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
        form: dict[str, JsonValue] = {}
        if data is not None:
            form.update(data)
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
    version = case["api_version"]
    base = case["api_base"]
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
