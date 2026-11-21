import json
from datetime import UTC, datetime
from typing import Final, Literal, TypedDict

import litellm
from litellm.cost_calculator import get_usage_object
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.proxy.common_request_processing import ProxyBaseLLMRequestProcessing
from litellm.types.utils import ModelResponse
from pydantic import BaseModel, JsonValue, TypeAdapter

# get_llm_provider without a model prints a banner to stdout and breaks the JSON answer
litellm.suppress_debug_info = True


class ChunkFields(TypedDict):
    kind: Literal["object", "text", "bytes"]
    value: dict[str, JsonValue] | None
    text: str | None


class Prices(TypedDict):
    recorder: float | None
    completion: float | None


class LogFields(TypedDict):
    model: str
    call_type: str
    details: dict[str, JsonValue]


class Input(TypedDict):
    chunk: ChunkFields
    enabled: bool
    model_name: str
    prices: Prices
    log: LogFields | None


class RecorderFields(TypedDict):
    model: str
    response_object: ModelResponse


class CompletionFields(TypedDict):
    completion_response: ModelResponse
    model: str
    service_tier: str | None


class CostUsage(TypedDict):
    cost: float


class CostEvent(TypedDict):
    usage: CostUsage


class ScriptedPrice:
    def __init__(self) -> None:
        self.cost: float = 0.0
        self.error: str | None = None


_RECORDER: Final[TypeAdapter[RecorderFields]] = TypeAdapter(RecorderFields)
_COMPLETION: Final[TypeAdapter[CompletionFields]] = TypeAdapter(CompletionFields)
_COST_EVENT: Final[TypeAdapter[CostEvent]] = TypeAdapter(CostEvent)
_JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
_RECORDER_PRICE: Final[ScriptedPrice] = ScriptedPrice()
_CALCULATOR_PRICE: Final[ScriptedPrice] = ScriptedPrice()
_OBSERVED: Final[list[JsonValue]] = []


def _respond(service: str, model: str, service_tier: str | None, response: object) -> JsonValue:
    usage = get_usage_object(response)
    if usage is None:
        return None
    return {
        "service": service,
        "model": model,
        "service_tier": service_tier,
        "usage": {
            "prompt_tokens": usage.prompt_tokens,
            "completion_tokens": usage.completion_tokens,
            "total_tokens": usage.total_tokens,
            "prompt_tokens_details": _details(usage.prompt_tokens_details),
            "completion_tokens_details": _details(usage.completion_tokens_details),
        },
    }


def _details(details: BaseModel | None) -> JsonValue:
    if details is None:
        return None
    return _JSON.validate_json(details.model_dump_json(exclude_none=True))


def _record_cost(**fields: JsonValue) -> float:
    parsed = _RECORDER.validate_python(fields)
    _OBSERVED.append(_respond("recorder", parsed["model"], None, parsed["response_object"]))
    if _RECORDER_PRICE.error is not None:
        raise RuntimeError(_RECORDER_PRICE.error)
    return _RECORDER_PRICE.cost


def _completion_cost(**fields: JsonValue) -> float:
    parsed = _COMPLETION.validate_python(fields)
    _OBSERVED.append(
        _respond(
            "calculator",
            parsed["model"],
            parsed["service_tier"],
            parsed["completion_response"],
        )
    )
    if _CALCULATOR_PRICE.error is not None:
        raise RuntimeError(_CALCULATOR_PRICE.error)
    return _CALCULATOR_PRICE.cost


litellm.response_cost_calculator = _record_cost
litellm.completion_cost = _completion_cost


def _load(prices: Prices) -> None:
    recorder = prices["recorder"]
    _RECORDER_PRICE.cost = 0.0 if recorder is None else recorder
    _RECORDER_PRICE.error = None if recorder is not None else "recorder pricing is unavailable"
    completion = prices["completion"]
    _CALCULATOR_PRICE.cost = 0.0 if completion is None else completion
    _CALCULATOR_PRICE.error = (
        None if completion is not None else "completion pricing is unavailable"
    )
    _OBSERVED.clear()


def _take_observed() -> list[JsonValue]:
    found = list(_OBSERVED)
    _OBSERVED.clear()
    return found


def _logging(fields: LogFields) -> Logging:
    log = Logging(
        model=fields["model"],
        messages=None,
        stream=True,
        call_type=fields["call_type"],
        start_time=datetime(2026, 1, 1, tzinfo=UTC),
        litellm_call_id="stream-usage-cost",
        function_id="stream-usage-cost",
    )
    litellm_params: dict[str, JsonValue] = {}
    optional_params: dict[str, JsonValue] = {}
    log.update_environment_variables(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/litellm_logging.py#L910
        litellm_params=litellm_params,
        optional_params=optional_params,
    )
    details: dict[str, JsonValue] = log.model_call_details
    details.update(fields["details"])
    return log


def _chunk(fields: ChunkFields) -> object:
    if fields["kind"] == "object":
        value = fields["value"]
        if value is None:
            raise ValueError("the object chunk needs a value")
        return value
    text = fields["text"]
    if text is None:
        raise ValueError("the text chunk needs a text")
    if fields["kind"] == "bytes":
        return text.encode("utf-8")
    return text


def _compact_json(payload: str) -> str:
    return json.dumps(_JSON.validate_json(payload), separators=(",", ":"))


def _compact(frame: str) -> str:
    compacted: list[str] = []
    for line in frame.split("\n"):
        stripped = line.strip()
        payload = stripped.split("data:", 1)[1].strip() if stripped.startswith("data:") else ""
        try:
            _COST_EVENT.validate_json(payload)
        except ValueError:
            compacted.append(line)
            continue
        suffix = "\r" if line.endswith("\r") else ""
        compacted.append("data: " + _compact_json(payload) + suffix)
    return "\n".join(compacted)


def _result_json(result: object) -> JsonValue:
    if isinstance(result, str):
        return _compact(result)
    if isinstance(result, bytes):
        return _compact(result.decode("utf-8"))
    return _JSON.validate_python(result)


def _debug(log: Logging | None) -> JsonValue:
    if log is None:
        return None
    details: dict[str, JsonValue] = log.model_call_details
    return _JSON.validate_python(details.get("response_cost_failure_debug_information"))


def _debug_present(log: Logging | None) -> bool:
    if log is None:
        return False
    details: dict[str, JsonValue] = log.model_call_details
    return "response_cost_failure_debug_information" in details


def run(input_value: Input) -> dict[str, JsonValue]:
    litellm.include_cost_in_streaming_usage = input_value["enabled"]
    _load(input_value["prices"])
    log_fields = input_value["log"]
    log = None if log_fields is None else _logging(log_fields)
    result = ProxyBaseLLMRequestProcessing._process_chunk_with_cost_injection(  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/common_request_processing.py#L4056-L4134
        _chunk(input_value["chunk"]),
        input_value["model_name"],
        log,
    )
    return {
        "result": _result_json(result),
        "debug": _debug(log),
        "debug_present": _debug_present(log),
        "calls": _take_observed(),
    }
