import os
from typing import Final, TypedDict

import httpx
from litellm.llms.custom_httpx.http_handler import HTTPHandler
from litellm.router_utils.add_retry_fallback_headers import get_hidden_params_dict
from litellm.types.utils import ImageResponse
from pydantic import JsonValue, TypeAdapter

from .models import Reply

COST_HEADER: Final = "llm_provider-x-litellm-response-cost"


class Hidden(TypedDict):
    additional_headers: dict[str, JsonValue]


BODY: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
HIDDEN: Final[TypeAdapter[Hidden]] = TypeAdapter(Hidden)


def prepare_environment() -> None:
    os.environ.clear()
    os.environ["AWS_CONFIG_FILE"] = os.devnull
    os.environ["AWS_SHARED_CREDENTIALS_FILE"] = os.devnull


class Fixture:
    def __init__(self, reply: Reply, extra_name: str | None) -> None:
        self.reply = reply
        self.extra_name = extra_name
        self.requests: list[httpx.Request] = []
        prepare_environment()

    def client(self) -> HTTPHandler:
        transport = httpx.MockTransport(self.respond)
        return HTTPHandler(client=httpx.Client(transport=transport))

    def respond(self, request: httpx.Request) -> httpx.Response:
        self.requests.append(request)
        return httpx.Response(
            self.reply.status,
            headers=self.reply.headers,
            content=self.reply.body.encode(),
        )


def header(request: httpx.Request, name: str) -> str | None:
    if name in request.headers:
        return request.headers[name]
    return None


def authorization(request: httpx.Request) -> JsonValue:
    value = header(request, "authorization")
    if value is None:
        return None
    if value.startswith("AWS4-HMAC-SHA256"):
        return "sigv4"
    return "other"


def extra_header(request: httpx.Request, name: str | None) -> JsonValue:
    if name is None:
        return None
    return header(request, name)


def request_view(fixture: Fixture) -> dict[str, JsonValue]:
    if not fixture.requests:
        return {
            "url": None,
            "body": None,
            "authorization": None,
            "extra_header": None,
            "posted": 0,
        }
    request = fixture.requests[0]
    return {
        "url": str(request.url),
        "body": BODY.validate_json(request.content),
        "authorization": authorization(request),
        "extra_header": extra_header(request, fixture.extra_name),
        "posted": len(fixture.requests),
    }


def images_of(response: ImageResponse) -> list[JsonValue]:
    data = response.data
    if data is None:
        return []
    return [image.b64_json for image in data if image.b64_json is not None]


def edit_cost(response: ImageResponse) -> JsonValue:
    hidden = HIDDEN.validate_python(get_hidden_params_dict(response))
    return hidden["additional_headers"].get(COST_HEADER)
