from dataclasses import dataclass, field

import httpx
import litellm
import openai
from openai.types.images_response import ImagesResponse
from pydantic import JsonValue as Json

from .probe import SUCCESS_MAX, SUCCESS_MIN, Answer, Probe

litellm.suppress_debug_info = True


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


@dataclass
class Fixture:
    probe: Probe
    calls: list[Call] = field(default_factory=list)

    def answer(self, params: dict[str, Json], timeout: float) -> Raw:
        self.calls.append(Call(body=dict(params), timeout=timeout))
        match self.probe.failure:
            case "timeout":
                raise httpx.ReadTimeout("read timed out")
            case "failed":
                raise httpx.ReadError("the transport failed")
            case None:
                return Raw(self.probe.reply)


class SyncImages:
    def __init__(self, fixture: Fixture) -> None:
        self.fixture = fixture

    @property
    def with_raw_response(self) -> "SyncImages":
        return self

    def generate(self, *, timeout: float, **params: Json) -> Raw:
        if timeout < 0:
            raise ValueError("timeout must not be negative")
        return self.fixture.answer(params, timeout)


class AsyncImages:
    def __init__(self, fixture: Fixture) -> None:
        self.fixture = fixture

    @property
    def with_raw_response(self) -> "AsyncImages":
        return self

    async def generate(
        self, *, timeout: float, **params: Json  # noqa: ASYNC109  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/openai.py#L1429
    ) -> Raw:
        if timeout < 0:
            raise ValueError("timeout must not be negative")
        return self.fixture.answer(params, timeout)


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


class Client:
    def __init__(self, fixture: Fixture, probe: Probe) -> None:
        self.api_key = probe.api_key
        self.base_url = api_base(probe)
        self.organization: str | None = None
        self.max_retries: int | None = None
        self.images: SyncImages | AsyncImages = (
            AsyncImages(fixture) if probe.is_async else SyncImages(fixture)
        )
