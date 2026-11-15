import asyncio
import logging
from typing import Literal, TypedDict

import httpx
from litellm.proxy.guardrails.guardrail_hooks.compresr.compresr import CompresrGuardrail, _query_for_target, _render_tool_intent, _replace_text_in_content, _safe_int, _safe_response_text  # noqa: E501  # isort: skip  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/compresr/compresr.py#L109-L246
from pydantic import JsonValue, TypeAdapter
from starlette.exceptions import HTTPException
from typing_extensions import override

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
Messages = list[dict[str, object]]
Fields = dict[str, object]


class Goal(TypedDict):
    context: str
    query: str


class Reply(TypedDict):
    status: int
    body: str


class Settings(TypedDict):
    api_base: str
    model: str
    target_compression_ratio: float
    coarse: bool
    dynamic: bool
    dynamic_min_ratio: float | None
    dynamic_max_ratio: float | None
    compression_params: Fields
    timeout: float | None
    fallback: Literal["fail_open", "fail_closed"]


class ValidateCall(TypedDict):
    function: Literal["validate_api_base"]
    settings: Settings


class QueryCall(TypedDict):
    function: Literal["query_for_target"]
    messages: Messages
    target_idx: int
    fallback: str


class IntentCall(TypedDict):
    function: Literal["render_tool_intent"]
    function_call: Fields


class FallbackCall(TypedDict):
    function: Literal["fallback_query"]
    messages: Messages


class RewriteCall(TypedDict):
    function: Literal["replace_text_in_content"]
    content: JsonValue
    new_text: str


class StatCall(TypedDict):
    function: Literal["safe_int"]
    value: JsonValue


class TextCall(TypedDict):
    function: Literal["safe_response_text"]
    text: str
    limit: int
    corrupt: bool


class CompressCall(TypedDict):
    function: Literal["compress"]
    settings: Settings
    goals: list[Goal]
    reply: Reply


Call = (
    ValidateCall
    | QueryCall
    | IntentCall
    | FallbackCall
    | RewriteCall
    | StatCall
    | TextCall
    | CompressCall
)


class Recorder:
    def __init__(self, reply: Reply) -> None:
        self.paths: list[JsonValue] = []
        self.bodies: list[JsonValue] = []
        self.headers: list[JsonValue] = []
        self.reply = reply

    def handle(self, request: httpx.Request) -> httpx.Response:
        self.paths.append(request.url.path)
        self.bodies.append(JSON.validate_json(request.content))
        sent = dict(request.headers.items())
        content_type = sent.get("content-type")
        api_key = sent.get("x-api-key")
        self.headers.append(
            {
                "content_type": "" if content_type is None else content_type,
                "api_key": "" if api_key is None else api_key,
            }
        )
        return httpx.Response(self.reply["status"], content=self.reply["body"].encode())


class Logs(logging.Filter):
    def __init__(self) -> None:
        super().__init__()
        self.records: list[JsonValue] = []

    @override
    def filter(self, record: logging.LogRecord) -> bool:
        text = record.getMessage().split(" detail=")[0].rstrip(". ")
        self.records.append({"level": record.levelname, "text": text})
        return True


def project(result: Fields) -> JsonValue:
    compressed = result.get("compressed_context")
    match compressed:
        case str():
            text: str | None = compressed
        case object():
            text = None
    return {
        "compressed_context": text,
        "original_tokens": _safe_int(result.get("original_tokens")),
        "compressed_tokens": _safe_int(result.get("compressed_tokens")),
    }


class ProbeMeta(type):
    pass


class Probe(CompresrGuardrail, metaclass=ProbeMeta):
    def compress(self, goals: list[Goal], reply: Reply) -> JsonValue:
        recorder = Recorder(reply)
        self.async_handler.client = httpx.AsyncClient(
            transport=httpx.MockTransport(recorder.handle)
        )
        contexts = [goal["context"] for goal in goals]
        queries = [goal["query"] for goal in goals]
        logs = Logs()
        logger = logging.getLogger("LiteLLM Proxy")
        logger.addFilter(logs)
        answer: dict[str, JsonValue] = {}
        try:
            results = asyncio.run(self._call_compress(contexts=contexts, queries=queries))
            if results is None:
                answer["outcome"] = "uncompressed"
            else:
                answer["outcome"] = "results"
                answer["results"] = [project(result) for result in results]
        except HTTPException as error:
            answer["outcome"] = "raised"
            answer["status"] = error.status_code
            answer["detail"] = JSON.validate_python(error.detail)
        finally:
            logger.removeFilter(logs)
        answer["path"] = recorder.paths
        answer["body"] = recorder.bodies
        answer["headers"] = recorder.headers
        answer["logs"] = logs.records
        return answer

    @staticmethod
    def fallback_of(messages: Messages) -> JsonValue:
        text, index = Probe._extract_fallback_query(messages)
        return {"query": text, "index": index}


def build(settings: Settings) -> Probe:
    return Probe(
        api_base=settings["api_base"],
        api_key="test-key",
        model=settings["model"],
        target_compression_ratio=settings["target_compression_ratio"],
        coarse=settings["coarse"],
        dynamic=settings["dynamic"],
        dynamic_min_ratio=settings["dynamic_min_ratio"],
        dynamic_max_ratio=settings["dynamic_max_ratio"],
        compression_params=settings["compression_params"],
        unreachable_fallback=settings["fallback"],
        timeout=settings["timeout"],
        enable_retrieval=False,
    )


def validate(input_value: ValidateCall) -> JsonValue:
    try:
        guardrail = build(input_value["settings"])
    except ValueError:
        return {"accepted": False}
    return {"accepted": True, "api_base": guardrail.compresr_api_base}


def intent(input_value: IntentCall) -> JsonValue:
    rendered = _render_tool_intent(input_value["function_call"])
    return {"intent": rendered}


def query(input_value: QueryCall) -> JsonValue:
    return {
        "query": _query_for_target(
            input_value["messages"], input_value["target_idx"], input_value["fallback"]
        )
    }


def rewrite(input_value: RewriteCall) -> JsonValue:
    replaced = _replace_text_in_content(input_value["content"], input_value["new_text"])
    return {"content": JSON.validate_python(replaced)}


def token_stat(input_value: StatCall) -> JsonValue:
    stat = _safe_int(input_value["value"])
    return {"value": stat}


class Undecodable(httpx.Response):
    @property
    @override
    def text(self) -> str:
        raise httpx.DecodingError("corrupt content-encoding")


def safe_text(input_value: TextCall) -> JsonValue:
    text = input_value["text"]
    if input_value["corrupt"]:
        response: httpx.Response = Undecodable(200)
    else:
        response = httpx.Response(200, content=text.encode())
    return {"text": _safe_response_text(response, input_value["limit"])}


def compress(input_value: CompressCall) -> JsonValue:
    guardrail = build(input_value["settings"])
    return guardrail.compress(input_value["goals"], input_value["reply"])


def run(input_value: Call) -> JsonValue:
    answer: JsonValue = None
    match input_value["function"]:
        case "validate_api_base":
            answer = validate(input_value)
        case "query_for_target":
            answer = query(input_value)
        case "render_tool_intent":
            answer = intent(input_value)
        case "fallback_query":
            answer = Probe.fallback_of(input_value["messages"])
        case "replace_text_in_content":
            answer = rewrite(input_value)
        case "safe_int":
            answer = token_stat(input_value)
        case "safe_response_text":
            answer = safe_text(input_value)
        case "compress":
            answer = compress(input_value)
    return answer
