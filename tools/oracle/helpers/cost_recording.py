from datetime import UTC, datetime
from typing import Final, Literal, TypedDict

import litellm
from litellm.cost_calculator import get_usage_object
from litellm.litellm_core_utils.core_helpers import set_response_cost_in_hidden_params
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.litellm_core_utils.llm_response_utils.response_metadata import ResponseMetadata
from litellm.router_utils.add_retry_fallback_headers import get_hidden_params_dict
from litellm.types.utils import ModelResponse, Usage
from pydantic import BaseModel, ConfigDict, Field, JsonValue, PrivateAttr, TypeAdapter


class UsageFields(TypedDict):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int | None


class CalculatorSpec(TypedDict):
    cost: float | None
    error: str | None


class TypedResponseFields(TypedDict):
    kind: Literal["typed"]
    model: str
    usage: UsageFields
    hidden_params: dict[str, JsonValue]
    background: bool


class GenerateResponseFields(TypedDict):
    kind: Literal["generate_content"]
    body: dict[str, JsonValue]


ResponseSpec = TypedResponseFields | GenerateResponseFields


class CallFields(TypedDict):
    response: ResponseSpec
    calculator: CalculatorSpec
    litellm_model_name: str | None
    router_model_id: str | None


class LogFields(TypedDict):
    model: str
    call_type: str
    litellm_params: dict[str, JsonValue]
    optional_params: dict[str, JsonValue]
    details: dict[str, JsonValue]


class LogCase(TypedDict):
    log: LogFields
    calls: list[CallFields]


class Input(TypedDict):
    function: Literal["record", "set_response_cost"]
    model_cost: dict[str, dict[str, JsonValue]]
    vertex_location: str | None
    ptu_cost_attribution: bool
    logs: list[LogCase]
    hidden_params: dict[str, JsonValue] | None
    cost: float | None


class SubjectResponse(BaseModel):
    model: str
    usage: Usage
    background: bool
    _hidden_params: dict[str, JsonValue] = PrivateAttr(default_factory=dict)

    def __init__(
        self, model: str, usage: Usage, *, background: bool, hidden: dict[str, JsonValue]
    ) -> None:
        super().__init__(model=model, usage=usage, background=background)
        self._hidden_params = hidden

    def hidden_params(self) -> dict[str, JsonValue]:
        return self._hidden_params

    def get(self, key: str) -> Usage | str | None:
        if key == "usage":
            return self.usage
        if key == "model":
            return self.model
        return None


class NativeBody(BaseModel):
    model_config = ConfigDict(populate_by_name=True)
    candidates: list[dict[str, JsonValue]]
    usage_metadata: dict[str, JsonValue] = Field(alias="usageMetadata")
    _hidden_params: dict[str, JsonValue] = PrivateAttr(default_factory=dict)

    def __init__(self, candidates: list[dict[str, JsonValue]], usage: dict[str, JsonValue]) -> None:
        super().__init__(candidates=candidates, usage_metadata=usage)
        self._hidden_params = {"additional_headers": {}}

    def hidden_params(self) -> dict[str, JsonValue]:
        return self._hidden_params


class CalculatorKwargs(TypedDict):
    model: str
    cache_hit: bool | None
    custom_llm_provider: str | None
    base_model: str | None
    call_type: str
    custom_pricing: bool
    prompt: str
    router_model_id: str | None
    data_residency: str | None
    vertex_location: str | None
    region_name: str | None
    response_object: SubjectResponse | ModelResponse


class HiddenCarrier(BaseModel):
    _hidden_params: dict[str, JsonValue] = PrivateAttr(default_factory=dict)

    def __init__(self, hidden: dict[str, JsonValue] | None) -> None:
        super().__init__()
        if hidden is not None:
            for name, value in hidden.items():
                self._hidden_params[name] = value


class CalculatorState:
    def __init__(self) -> None:
        self.cost: float = 0.0
        self.error: str | None = None
        self.observed: list[JsonValue] = []


_CALCULATOR: Final[TypeAdapter[CalculatorKwargs]] = TypeAdapter(CalculatorKwargs)
_JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
_AMOUNT: Final[TypeAdapter[float | None]] = TypeAdapter(float | None)
_METADATA: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])
_CANDIDATES: Final[TypeAdapter[list[dict[str, JsonValue]]]] = TypeAdapter(
    list[dict[str, JsonValue]]
)
_STATE: Final[CalculatorState] = CalculatorState()


def _calculator(**fields: JsonValue) -> float:
    observation = _CALCULATOR.validate_python(fields)
    _STATE.observed.append(_observe(observation))
    if _STATE.error is not None:
        raise RuntimeError(_STATE.error)
    return _STATE.cost


litellm.response_cost_calculator = _calculator


def _observe(fields: CalculatorKwargs) -> dict[str, JsonValue]:
    usage = get_usage_object(fields["response_object"])
    counts: JsonValue = None
    if usage is not None:
        prompt: JsonValue = usage.prompt_tokens
        completion: JsonValue = usage.completion_tokens
        parts: list[JsonValue] = [prompt, completion]
        counts = _JSON.validate_python(parts)
    return {
        "model": fields["model"],
        "cache_hit": fields["cache_hit"],
        "provider": fields["custom_llm_provider"],
        "base_model": fields["base_model"],
        "call_type": fields["call_type"],
        "custom_pricing": fields["custom_pricing"],
        "prompt": fields["prompt"],
        "router_model_id": fields["router_model_id"],
        "data_residency": fields["data_residency"],
        "vertex_location": fields["vertex_location"],
        "region_name": fields["region_name"],
        "usage": counts,
    }


def _load(spec: CalculatorSpec) -> None:
    _STATE.error = spec["error"]
    cost = spec["cost"]
    _STATE.cost = 0.0 if cost is None else cost
    _STATE.observed = []


def _take_observed() -> list[JsonValue]:
    found = _STATE.observed
    _STATE.observed = []
    return found


def _usage_fields(spec: UsageFields) -> Usage:
    total = spec["total_tokens"]
    if total is None:
        total = spec["prompt_tokens"] + spec["completion_tokens"]
    return Usage(
        prompt_tokens=spec["prompt_tokens"],
        completion_tokens=spec["completion_tokens"],
        total_tokens=total,
    )


def _hidden_seed(hidden: dict[str, JsonValue]) -> dict[str, JsonValue]:
    seed = dict(hidden)
    if not seed:
        seed["additional_headers"] = {}
    return seed


def _response_of(spec: ResponseSpec) -> SubjectResponse | NativeBody:
    if spec["kind"] == "typed":
        return SubjectResponse(
            spec["model"],
            _usage_fields(spec["usage"]),
            background=spec["background"],
            hidden=_hidden_seed(spec["hidden_params"]),
        )
    body = spec["body"]
    return NativeBody(
        _CANDIDATES.validate_python(body.get("candidates")),
        _METADATA.validate_python(body.get("usageMetadata")),
    )


def _logging(case: LogFields) -> Logging:
    log = Logging(
        model=case["model"],
        messages=None,
        stream=False,
        call_type=case["call_type"],
        start_time=datetime(2026, 1, 1, tzinfo=UTC),
        litellm_call_id="cost-recording-case",
        function_id="cost-recording-case",
    )
    log.update_environment_variables(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/litellm_logging.py#L910
        litellm_params=case["litellm_params"],
        optional_params=case["optional_params"],
    )
    details: dict[str, JsonValue] = log.model_call_details
    details.update(case["details"])
    return log


def _record_call(log: Logging, call: CallFields) -> dict[str, JsonValue]:
    _load(call["calculator"])
    response = _response_of(call["response"])
    kwargs: dict[str, JsonValue] = {"model_info": {"id": call["router_model_id"]}}
    ResponseMetadata(response).set_hidden_params(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/llm_response_utils/response_metadata.py#L118
        log, call["litellm_model_name"], kwargs
    )
    details: dict[str, JsonValue] = log.model_call_details
    return {
        "amount": _AMOUNT.validate_python(response.hidden_params().get("response_cost")),
        "failure": _JSON.validate_json(
            _JSON.dump_json(details.get("response_cost_failure_debug_information"))
        ),
        "diagnostic": _JSON.validate_json(_JSON.dump_json(details.get("zero_cost_diagnostic"))),
        "warned": log.zero_cost_warned,
        "observed": _take_observed(),
    }


def _set_cost(input_value: Input) -> JsonValue:
    carrier = HiddenCarrier(input_value["hidden_params"])
    set_response_cost_in_hidden_params(carrier, input_value["cost"])
    return _JSON.validate_python(get_hidden_params_dict(carrier))


def run(input_value: Input) -> dict[str, JsonValue]:
    if input_value["function"] == "set_response_cost":
        return {"hidden_params": _set_cost(input_value)}
    entries: dict[str, dict[str, JsonValue]] = {
        name: dict(fields) for name, fields in input_value["model_cost"].items()
    }
    litellm.model_cost = entries
    litellm.vertex_location = input_value["vertex_location"]
    logs: list[JsonValue] = []
    for case in input_value["logs"]:
        log = _logging(case["log"])
        calls: list[JsonValue] = [_record_call(log, call) for call in case["calls"]]
        logs.append({"calls": calls})
    return {"logs": logs}
