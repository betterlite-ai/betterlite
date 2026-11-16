import logging
import zlib
from datetime import UTC, datetime
from typing import Annotated, Final, Literal

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.passthrough.transformation import PassthroughStreamCollector
from litellm.llms.bedrock.passthrough.transformation import BedrockPassthroughConfig
from litellm.types.utils import CostResponseTypes
from pydantic import BaseModel, ConfigDict, Field, TypeAdapter
from pydantic import JsonValue as Json

CONFIG: Final[BedrockPassthroughConfig] = BedrockPassthroughConfig()
API_BASE: Final = "https://bedrock-runtime.us-east-1.amazonaws.com"
SPEND_WARNING: Final = "Bedrock passthrough spend tracking skipped:"


class Closed(BaseModel):
    model_config = ConfigDict(extra="forbid")


class Frame(Closed):
    event_type: str
    payload: str


class UrlRun(Closed):
    model: str
    endpoint: str
    region: str | None = None
    model_id: str | None = None
    api_base: str | None = None
    runtime_endpoint: str | None = None
    query: dict[str, str] | None = None


class SignRun(Closed):
    headers: dict[str, str] = Field(default_factory=dict)
    data: dict[str, Json] | None = None


class ReplyRun(Closed):
    provider: str
    endpoint: str


class SelectionRun(Closed):
    model: str
    endpoint: str


class UrlInput(Closed):
    kind: Literal["url"]
    environment: dict[str, str] = Field(default_factory=dict)
    runs: list[UrlRun]


class SignInput(Closed):
    kind: Literal["sign"]
    bearer: str | None = None
    runs: list[SignRun]


class ReplyInput(Closed):
    kind: Literal["reply"]
    runs: list[ReplyRun]


class CollectorInput(Closed):
    kind: Literal["collector"]
    model: str
    endpoint: str
    split: bool
    frames: list[Frame]


class SelectionInput(Closed):
    kind: Literal["selection"]
    runs: list[SelectionRun]
    frames: list[Frame]


Request = Annotated[UrlInput | SignInput | ReplyInput | CollectorInput | SelectionInput, Field(discriminator="kind")]
REQUEST: Final[TypeAdapter[Request]] = TypeAdapter(Request)


class MessageDump(BaseModel):
    model_config = ConfigDict(extra="ignore")

    role: Json = None
    content: Json = None


class ChoiceDump(BaseModel):
    model_config = ConfigDict(extra="ignore")

    message: MessageDump = Field(default_factory=MessageDump)
    finish_reason: Json = None


class UsageDump(BaseModel):
    model_config = ConfigDict(extra="ignore")

    prompt_tokens: Json = None
    completion_tokens: Json = None
    total_tokens: Json = None


class ResponseDump(BaseModel):
    model_config = ConfigDict(extra="ignore")

    model: Json = None
    choices: list[ChoiceDump] = Field(default_factory=list)
    usage: UsageDump | None = None


DUMP: Final[TypeAdapter[ResponseDump]] = TypeAdapter(ResponseDump)


class WarningCapture(logging.Handler):
    def __init__(self) -> None:
        super().__init__()
        self.messages: list[str] = []

    def emit(self, record: logging.LogRecord) -> None:
        self.messages.append(record.getMessage())


def logging_fixture() -> Logging:
    log = Logging(
        model="anthropic.claude-3-sonnet-20240229-v1:0",
        messages=None,
        stream=False,
        call_type="completion",
        start_time=datetime(2024, 1, 1, tzinfo=UTC),
        litellm_call_id="bedrock-services-passthrough",
        function_id="bedrock-services-passthrough",
    )
    details: dict[str, Json] = {"custom_llm_provider": "bedrock", "cache_hit": False}
    log.model_call_details = details
    return log


def params_of(run: UrlRun) -> dict[str, Json]:
    params: dict[str, Json] = {}
    if run.region is not None:
        params["aws_region_name"] = run.region
    if run.model_id is not None:
        params["model_id"] = run.model_id
    if run.runtime_endpoint is not None:
        params["aws_bedrock_runtime_endpoint"] = run.runtime_endpoint
    return params


def url_run(run: UrlRun) -> Json:
    body: dict[str, Json] = {"stream": True}
    url, endpoint_url = CONFIG.get_complete_url(
        api_base=run.api_base,
        api_key=None,
        model=run.model,
        endpoint=run.endpoint,
        request_query_params=run.query,
        litellm_params=params_of(run),
    )
    streaming = CONFIG.is_streaming_request(run.endpoint, body)
    return {"url": str(url), "endpoint_url": endpoint_url, "streaming": streaming}


def url_runs(runs: list[UrlRun]) -> Json:
    return {"runs": [url_run(run) for run in runs]}


def sign_run(run: SignRun) -> Json:
    signed, body = CONFIG.sign_request(
        headers=dict(run.headers),
        litellm_params={"api_key": "sk-ignored"},
        request_data=run.data,
        api_base=API_BASE,
        model="m",
    )
    text = "" if body is None else body.decode("utf-8")
    pairs: list[Json] = []
    for name in sorted(signed):
        pairs.append({"name": name, "value": signed[name]})
    return {"headers": pairs, "body": text}


def sign_runs(runs: list[SignRun]) -> Json:
    return {"runs": [sign_run(run) for run in runs]}


def reply_run(run: ReplyRun) -> Json:
    response = httpx.Response(200, content=b"{}")
    try:
        logged = CONFIG.logging_non_streaming_response(
            model="m",
            custom_llm_provider=run.provider,
            httpx_response=response,
            request_data={},
            logging_obj=logging_fixture(),
            endpoint=run.endpoint,
        )
    except ValueError:
        return {"logged": None, "raised": True}
    if logged is None:
        return {"logged": None, "raised": False}
    return {"logged": True, "raised": False}


def reply_runs(runs: list[ReplyRun]) -> Json:
    return {"runs": [reply_run(run) for run in runs]}


def header(name: str, value: str) -> bytes:
    length = len(value).to_bytes(2, "big")
    return bytes([len(name)]) + name.encode() + bytes([7]) + length + value.encode()


def frame(entry: Frame) -> bytes:
    headers = (
        header(":event-type", entry.event_type)
        + header(":content-type", "application/json")
        + header(":message-type", "event")
    )
    payload = entry.payload.encode()
    total = 16 + len(headers) + len(payload)
    prelude = total.to_bytes(4, "big") + len(headers).to_bytes(4, "big")
    body = prelude + zlib.crc32(prelude).to_bytes(4, "big") + headers + payload
    return body + zlib.crc32(body).to_bytes(4, "big")


def frames_bytes(frames: list[Frame]) -> bytes:
    return b"".join(frame(entry) for entry in frames)


def feed(collector: PassthroughStreamCollector, data: bytes, split: bool) -> None:
    midpoint = len(data) // 2
    if split and midpoint > 0:
        collector.add(data[:midpoint])
        collector.add(data[midpoint:])
        return
    collector.add(data)


def response_view(response: CostResponseTypes) -> Json:
    dump = DUMP.validate_json(response.model_dump_json())
    choice = ChoiceDump()
    if dump.choices:
        choice = dump.choices[0]
    usage = dump.usage
    view: dict[str, Json] = {
        "model": dump.model,
        "role": choice.message.role,
        "content": choice.message.content,
        "finish_reason": choice.finish_reason,
    }
    view["usage"] = None
    if usage is not None:
        view["usage"] = {
            "prompt_tokens": usage.prompt_tokens,
            "completion_tokens": usage.completion_tokens,
            "total_tokens": usage.total_tokens,
        }
    return view


def collector_result(model: str, endpoint: str, split: bool, frames: list[Frame]) -> tuple[bool, Json]:
    collector = CONFIG.create_stream_collector(model=model, custom_llm_provider="bedrock", endpoint=endpoint)
    feed(collector, frames_bytes(frames), split)
    logged = collector.build_logged_response(logging_fixture())
    if logged is None:
        return False, None
    return True, response_view(logged)


def warning_of(messages: list[str]) -> Json:
    for message in messages:
        if message.startswith(SPEND_WARNING):
            return message
    return None


def selection_run(run: SelectionRun, frames: list[Frame]) -> Json:
    capture = WarningCapture()
    logger = logging.getLogger("LiteLLM")
    logger.addHandler(capture)
    try:
        outcome = collector_result(run.model, run.endpoint, False, frames)
    finally:
        logger.removeHandler(capture)
    return {"decoded": outcome[0], "warning": warning_of(capture.messages)}


def selection_runs(runs: list[SelectionRun], frames: list[Frame]) -> Json:
    return {"runs": [selection_run(run, frames) for run in runs]}


def run(input_value: dict[str, Json]) -> Json:
    request = REQUEST.validate_python(input_value)
    match request:
        case UrlInput():
            return url_runs(request.runs)
        case SignInput():
            return sign_runs(request.runs)
        case ReplyInput():
            return reply_runs(request.runs)
        case CollectorInput():
            decoded, view = collector_result(request.model, request.endpoint, request.split, request.frames)
            return {"runs": [{"decoded": decoded, "view": view}]}
        case SelectionInput():
            return selection_runs(request.runs, request.frames)
    raise ValueError("unknown bedrock passthrough request")
