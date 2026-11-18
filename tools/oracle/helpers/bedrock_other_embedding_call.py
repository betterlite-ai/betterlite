import json
import os
from dataclasses import dataclass, field
from datetime import UTC, datetime
from functools import partial
from typing import Literal, NotRequired, Protocol, TypedDict, cast

import botocore.auth  # type: ignore[import-untyped]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/embed/embedding.py#L51-L62
import httpx
import litellm
from litellm.llms.bedrock.common_utils import BedrockError
from litellm.llms.bedrock.embed import embedding as embedding_module
from litellm.llms.custom_httpx.http_handler import HTTPHandler
from litellm.types.utils import EmbeddingResponse
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter


class Reply(TypedDict):
    status: int
    body: str
    headers: NotRequired[dict[str, str]]
    failure: NotRequired[Literal["timeout", "failed"]]


class Case(TypedDict):
    function: str
    model: str
    input: list[str]
    now: int
    optional_params: NotRequired[dict[str, JsonValue]]
    api_base: NotRequired[str | None]
    extra_headers: NotRequired[dict[str, str] | None]
    api_key: NotRequired[str | None]
    timeout: NotRequired[float | None]
    aembedding: NotRequired[bool]
    drop_params: NotRequired[bool]
    environment: NotRequired[dict[str, str]]
    replies: NotRequired[list[Reply]]
    supplied: NotRequired[Literal["none", "sync", "async"]]


class Record(TypedDict):
    object: str
    index: int
    embedding: list[JsonValue] | str


class Usage(TypedDict):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int
    prompt_tokens_details: dict[str, JsonValue] | None


class Payload(TypedDict):
    object: str
    data: list[Record]
    model: str | None
    usage: Usage


class TimeoutView(BaseModel):
    model_config = ConfigDict(from_attributes=True, arbitrary_types_allowed=True)
    connect: float
    read: float
    write: float
    pool: float


class Tokenizer(Protocol):
    def encode(self, text: str, /) -> list[int]: ...


class FixedTokenizer(Tokenizer):
    def encode(self, text: str, /) -> list[int]:
        return [ord(char) for char in text]


class Calls(Protocol):
    phases: list[str]

    def pre_call(self, **arguments: JsonValue) -> None: ...

    def post_call(self, **arguments: JsonValue) -> None: ...


class Logger(Calls):
    def __init__(self) -> None:
        self.phases: list[str] = []

    def pre_call(self, **arguments: JsonValue) -> None:
        self.phases.append("pre_api_call")

    def post_call(self, **arguments: JsonValue) -> None:
        self.phases.append("post_api_call")


CASE: TypeAdapter[Case] = TypeAdapter(Case)
PAYLOAD: TypeAdapter[Payload] = TypeAdapter(Payload)
BODY: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)


SKIPPED = frozenset(
    {"host", "content-length", "user-agent", "accept-encoding", "connection", "transfer-encoding"}
)


def signature_scope(value: str) -> str:
    parts = value.split(", Signature=")
    return parts[0]


def request_value(request: httpx.Request) -> JsonValue:
    headers: dict[str, JsonValue] = {}
    for name, value in request.headers.items():
        lowered = name.lower()
        if lowered in SKIPPED or (lowered == "accept" and value == "*/*"):
            continue
        headers[lowered] = signature_scope(value)
    return {
        "url": str(request.url),
        "headers": headers,
        "body": BODY.validate_json(request.content),
    }


def timeout_fields(timeout: JsonValue) -> list[JsonValue] | None:
    if timeout is None:
        return None
    record = TimeoutView.model_validate(timeout)
    return [record.connect, record.read, record.write, record.pool]


@dataclass
class Fixture:
    replies: list[Reply]
    now: int
    requests: list[JsonValue] = field(default_factory=list)
    factory: list[JsonValue] = field(default_factory=list)

    def answer(self, request: httpx.Request) -> httpx.Response:
        self.requests.append(request_value(request))
        if not self.replies:
            raise httpx.ReadError("the reply queue is empty")
        reply = self.replies.pop(0)
        failure = reply.get("failure")
        if failure == "timeout":
            raise httpx.ReadTimeout("read timed out")
        if failure == "failed":
            raise httpx.ReadError("the transport failed")
        return httpx.Response(
            reply["status"], headers=reply.get("headers"), content=reply["body"].encode()
        )

    def client(self) -> HTTPHandler:
        transport = httpx.MockTransport(self.answer)
        return HTTPHandler(client=httpx.Client(transport=transport, follow_redirects=True))

    def sync_client(self, params: dict[str, JsonValue] | None = None) -> HTTPHandler:
        timeout: JsonValue = None if params is None else params.get("timeout")
        record = {"kind": "sync", "provider": None, "timeout": timeout_fields(timeout)}
        self.factory.append(record)
        return self.client()


def configure(fixture: Fixture) -> None:
    embedding_module._get_httpx_client = fixture.sync_client  # type: ignore[attr-defined]  # noqa: SLF001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/embed/embedding.py#L119
    botocore.auth.get_current_datetime = partial(datetime.fromtimestamp, fixture.now, tz=UTC)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/embed/embedding.py#L60


def set_environment(case: Case) -> None:
    environment = case.get("environment")
    os.environ.clear()
    if environment is not None:
        os.environ.update(environment)


def supplied(case: Case, fixture: Fixture) -> HTTPHandler | None:
    if case.get("supplied") == "sync":
        return fixture.client()
    return None


def usage_value(usage: Usage) -> JsonValue:
    details = usage["prompt_tokens_details"]
    counts: dict[str, JsonValue] | None = None
    if details is not None:
        counts = {name: value for name, value in details.items() if value is not None}
    return {
        "prompt_tokens": usage["prompt_tokens"],
        "completion_tokens": usage["completion_tokens"],
        "total_tokens": usage["total_tokens"],
        "prompt_tokens_details": counts,
    }


def response_value(response: EmbeddingResponse) -> JsonValue:
    payload = PAYLOAD.validate_json(response.model_dump_json())
    hidden = cast(dict[str, JsonValue], response._hidden_params)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2311
    return {
        "object": payload["object"],
        "data": [
            {"object": r["object"], "index": r["index"], "embedding": r["embedding"]}
            for r in payload["data"]
        ],
        "model": payload["model"],
        "usage": usage_value(payload["usage"]),
        "invocation_arn": hidden.get("_invocation_arn"),
    }


def probe(case: Case) -> JsonValue:
    set_environment(case)
    raw = case.get("replies")
    fixture = Fixture([] if raw is None else list(raw), case["now"])
    configure(fixture)
    values = case.get("optional_params")
    optional_params: dict[str, JsonValue] = {} if values is None else dict(values)
    logger = Logger()
    ok = False
    kind: str | None = None
    status: int | None = None
    message: str | None = None
    response: JsonValue = None
    try:
        reply = embedding_module.BedrockEmbedding().embeddings(
            model=case["model"],
            input=list(case["input"]),
            api_base=case.get("api_base"),
            model_response=EmbeddingResponse(),
            print_verbose=print,
            encoding=FixedTokenizer(),
            logging_obj=logger,
            client=supplied(case, fixture),
            timeout=case.get("timeout"),
            aembedding=False,
            extra_headers=case.get("extra_headers"),
            optional_params=optional_params,
            litellm_params={"drop_params": case.get("drop_params")},
            api_key=case.get("api_key"),
        )
    except BedrockError as error:
        kind = "http"
        status = error.status_code
    except litellm.Timeout:
        kind = "bedrock"
        status = 408
    except httpx.TimeoutException:
        kind = "bedrock"
        status = 408
    except httpx.HTTPStatusError as error:
        kind = "http"
        status = error.response.status_code
    except httpx.TransportError:
        kind = "transport"
    except json.JSONDecodeError:
        kind = "decode"
    except Exception as error:  # noqa: BLE001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/embed/embedding.py#L490
        kind = "provider"
        message = str(error)
    else:
        ok = True
        kind = "ok"
        response = response_value(reply)
    return {
        "ok": ok,
        "status": status,
        "kind": kind,
        "message": message,
        "requests": fixture.requests,
        "factory": fixture.factory,
        "response": response,
        "phases": logger.phases,
        "remaining": optional_params,
    }


def run(input_value: dict[str, JsonValue]) -> JsonValue:
    return probe(CASE.validate_python(input_value))
