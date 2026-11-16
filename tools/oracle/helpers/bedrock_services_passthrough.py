import logging
from datetime import UTC, datetime
from typing import Final, Literal, NotRequired, Protocol, TypedDict, override

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.bedrock.common_utils import BedrockError
from litellm.llms.bedrock.passthrough.transformation import BedrockPassthroughConfig
from pydantic import BaseModel, JsonValue, TypeAdapter

API_BASE: Final = "https://bedrock-runtime.us-east-1.amazonaws.com"
SPEND_WARNING: Final = "Bedrock passthrough spend tracking skipped:"


class Typed(Protocol):
    def sign_request(
        self,
        headers: dict[str, str],
        litellm_params: dict[str, JsonValue],
        request_data: dict[str, JsonValue] | None,
        api_base: str,
        model: str | None = None,
    ) -> tuple[dict[str, str], bytes | None]: ...


CONFIG: Final[BedrockPassthroughConfig] = BedrockPassthroughConfig()
SIGNER: Typed = CONFIG


class UrlRun(TypedDict):
    model: str
    endpoint: str
    region: NotRequired[str | None]
    model_id: NotRequired[str | None]
    api_base: NotRequired[str | None]
    runtime_endpoint: NotRequired[str | None]
    query: NotRequired[dict[str, str] | None]


class SignRun(TypedDict):
    headers: dict[str, str]
    data: NotRequired[dict[str, JsonValue] | None]


class ReplyRun(TypedDict):
    provider: str
    endpoint: str
    body: str


class CollectRun(TypedDict):
    model: str
    endpoint: str


class UrlCall(TypedDict):
    kind: Literal["url"]
    runs: list[UrlRun]


class SignCall(TypedDict):
    kind: Literal["sign"]
    bearer: str | None
    runs: list[SignRun]


class ReplyCall(TypedDict):
    kind: Literal["reply"]
    runs: list[ReplyRun]


class CollectCall(TypedDict):
    kind: Literal["collect"]
    runs: list[CollectRun]
    stream: str


Input = UrlCall | SignCall | ReplyCall | CollectCall


class Message(TypedDict):
    content: JsonValue


class Choice(TypedDict):
    message: Message
    finish_reason: JsonValue


class Usage(TypedDict):
    prompt_tokens: JsonValue
    completion_tokens: JsonValue
    total_tokens: JsonValue


class Reply(TypedDict):
    model: JsonValue
    choices: list[Choice]
    usage: NotRequired[Usage | None]


REPLY: Final[TypeAdapter[Reply]] = TypeAdapter(Reply)


class WarningCapture(logging.Handler):
    def __init__(self) -> None:
        super().__init__()
        self.messages: list[str] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        self.messages.append(record.getMessage())


def logging_fixture(model: str) -> Logging:
    log = Logging(
        model=model,
        messages=None,
        stream=False,
        call_type="completion",
        start_time=datetime(2024, 1, 1, tzinfo=UTC),
        litellm_call_id="bedrock-services-passthrough",
        function_id="bedrock-services-passthrough",
    )
    details: dict[str, JsonValue] = {"custom_llm_provider": "bedrock", "cache_hit": False}
    log.model_call_details = details
    return log


def params_of(
    region: str | None, model_id: str | None, runtime_endpoint: str | None
) -> dict[str, JsonValue]:
    params: dict[str, JsonValue] = {}
    if region is not None:
        params["aws_region_name"] = region
    if model_id is not None:
        params["model_id"] = model_id
    if runtime_endpoint is not None:
        params["aws_bedrock_runtime_endpoint"] = runtime_endpoint
    return params


def url_run(run: UrlRun) -> JsonValue:
    request_data: dict[str, JsonValue] = {}
    url, endpoint_url = CONFIG.get_complete_url(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/passthrough/transformation.py#L171-L178
        api_base=run.get("api_base"),
        api_key=None,
        model=run["model"],
        endpoint=run["endpoint"],
        request_query_params=run.get("query"),
        litellm_params=params_of(
            run.get("region"), run.get("model_id"), run.get("runtime_endpoint")
        ),
    )
    return {
        "url": str(url),
        "endpoint_url": endpoint_url,
        "streaming": CONFIG.is_streaming_request(run["endpoint"], request_data),  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/passthrough/transformation.py#L136-L137
    }


def sign_run(run: SignRun) -> JsonValue:
    signed, body = SIGNER.sign_request(
        headers=run["headers"],
        litellm_params={"api_key": "sk-ignored"},
        request_data=run.get("data"),
        api_base=API_BASE,
        model="m",
    )
    text = "" if body is None else body.decode("utf-8")
    pairs: list[JsonValue] = [
        {"name": name, "value": value} for name, value in sorted(signed.items())
    ]
    return {"headers": pairs, "body": text}


def reply_run(run: ReplyRun) -> JsonValue:
    request_data: dict[str, JsonValue] = {}
    response = httpx.Response(200, content=run["body"].encode())
    try:
        logged = CONFIG.logging_non_streaming_response(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/passthrough/transformation.py#L234-L275
            model="m",
            custom_llm_provider=run["provider"],
            httpx_response=response,
            request_data=request_data,
            logging_obj=logging_fixture("m"),
            endpoint=run["endpoint"],
        )
    except (ValueError, BedrockError):
        return {"logged": False, "raised": True}
    return {"logged": logged is not None, "raised": False}


def response_view(response: BaseModel) -> JsonValue:
    reply = REPLY.validate_json(response.model_dump_json())
    usage = reply.get("usage")
    counted: JsonValue = None
    if usage is not None:
        counted = {
            "prompt_tokens": usage["prompt_tokens"],
            "completion_tokens": usage["completion_tokens"],
            "total_tokens": usage["total_tokens"],
        }
    first = reply["choices"][0]
    return {
        "model": reply["model"],
        "content": first["message"]["content"],
        "finish_reason": first["finish_reason"],
        "usage": counted,
    }


def warning_of(messages: list[str]) -> JsonValue:
    for message in messages:
        if message.startswith(SPEND_WARNING):
            return message
    return None


def collect_run(run: CollectRun, data: bytes) -> JsonValue:
    capture = WarningCapture()
    logger = logging.getLogger("LiteLLM")
    logger.addHandler(capture)
    try:
        collector = CONFIG.create_stream_collector(
            model=run["model"], custom_llm_provider="bedrock", endpoint=run["endpoint"]
        )
    finally:
        logger.removeHandler(capture)
    middle = len(data) // 2
    collector.add(data[:middle])
    collector.add(data[middle:])
    logged = collector.build_logged_response(logging_fixture(run["model"]))
    warning = warning_of(capture.messages)
    if logged is None:
        return {"decoded": False, "view": None, "warning": warning}
    return {"decoded": True, "view": response_view(logged), "warning": warning}


def run(input_value: Input) -> JsonValue:
    if input_value["kind"] == "url":
        return {"runs": [url_run(item) for item in input_value["runs"]]}
    if input_value["kind"] == "sign":
        return {"runs": [sign_run(item) for item in input_value["runs"]]}
    if input_value["kind"] == "reply":
        return {"runs": [reply_run(item) for item in input_value["runs"]]}
    data = bytes.fromhex(input_value["stream"])
    return {"runs": [collect_run(item, data) for item in input_value["runs"]]}
