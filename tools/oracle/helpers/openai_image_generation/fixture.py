from dataclasses import dataclass, field
from typing import Final, Self, TypedDict

import httpx
import litellm
import openai
from openai.types.images_response import ImagesResponse
from pydantic import JsonValue as Json
from pydantic import TypeAdapter

from .probe import SUCCESS_MAX, SUCCESS_MIN, Answer, Probe

litellm.suppress_debug_info = True


class Extras(TypedDict, total=False):
    extra_headers: dict[str, str]
    extra_query: dict[str, str]
    extra_body: dict[str, Json]


EXTRAS: Final = TypeAdapter(Extras)
EXTRA_NAMES: Final = frozenset({"extra_headers", "extra_query", "extra_body"})


class Raw:
    def __init__(self, answer: Answer) -> None:
        self.answer = answer

    @property
    def headers(self) -> dict[str, str]:
        return self.answer.headers

    def parse(self) -> ImagesResponse:
        response = httpx.Response(
            self.answer.status,
            headers=self.answer.headers,
            content=self.answer.body.encode(),
            request=httpx.Request("POST", "https://example.test/"),
        )
        if self.answer.status < SUCCESS_MIN or self.answer.status >= SUCCESS_MAX:
            raise openai.APIStatusError(
                "the images endpoint failed", response=response, body=None
            )
        return ImagesResponse.model_validate_json(self.answer.body)


@dataclass
class Call:
    body: dict[str, Json]
    timeout: float
    url: str
    headers: dict[str, Json]
    query: dict[str, Json]


@dataclass
class Fixture:
    probe: Probe
    calls: list[Call] = field(default_factory=list)

    def answer(self, call: Call) -> Raw:
        self.calls.append(call)
        match self.probe.failure:
            case "timeout":
                raise httpx.ReadTimeout("read timed out")
            case "failed":
                raise httpx.ReadError("the transport failed")
            case None:
                return Raw(self.probe.reply)


class Images:
    def __init__(self, fixture: Fixture, client: "Client") -> None:
        self.fixture = fixture
        self.client = client

    @property
    def with_raw_response(self) -> Self:
        return self

    def send(self, timeout: float, params: dict[str, Json]) -> Raw:
        if timeout < 0:
            raise ValueError("timeout must not be negative")
        return self.fixture.answer(self.client.wire_call(params, timeout))


class SyncImages(Images):
    def generate(self, *, timeout: float, **params: Json) -> Raw:
        return self.send(timeout, params)


class AsyncImages(Images):
    async def generate(
        self, *, timeout: float, **params: Json  # noqa: ASYNC109  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/openai.py#L1429
    ) -> Raw:
        return self.send(timeout, params)


def api_base(probe: Probe) -> str:
    found = probe.api_base
    if found is None:
        raise ValueError("a request probe needs api_base")
    return found


def timeout_value(probe: Probe) -> float:
    found = probe.timeout
    if found is None:
        raise ValueError("a request probe needs timeout")
    return float(found)


class BaseUrl:
    def __init__(self, reference: str) -> None:
        self._uri_reference = reference


class Client:
    def __init__(self, fixture: Fixture, probe: Probe) -> None:
        self.api_key = probe.api_key
        self.base_url = api_base(probe)
        self._base_url = BaseUrl(self.base_url)
        self.organization: str | None = None
        self.max_retries: int | None = None
        self.images: SyncImages | AsyncImages = (
            AsyncImages(fixture, self) if probe.is_async else SyncImages(fixture, self)
        )

    def wire_call(self, params: dict[str, Json], timeout: float) -> Call:
        extras = EXTRAS.validate_python(params)
        headers: dict[str, Json] = {
            "content-type": "application/json",
            "authorization": f"Bearer {self.api_key}",
        }
        if self.organization is not None:
            headers["openai-organization"] = self.organization
        if "extra_headers" in extras:
            headers.update(
                {name.lower(): value for name, value in extras["extra_headers"].items()}
            )
        body = {name: value for name, value in params.items() if name not in EXTRA_NAMES}
        if "extra_body" in extras:
            body.update(extras["extra_body"])
        query: dict[str, Json] = {}
        if "extra_query" in extras:
            query.update(extras["extra_query"])
        url = f"{self.base_url.rstrip('/')}/images/generations"
        return Call(body=body, timeout=timeout, url=url, headers=headers, query=query)
