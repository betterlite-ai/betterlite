from collections.abc import Callable
from typing import Final, Literal, NotRequired, TypeAlias, TypeVar, TypedDict, cast

from litellm.litellm_core_utils.litellm_logging import Logging as LiteLLMLoggingObj
from litellm.responses import streaming_iterator as primitives
from litellm.responses.streaming_iterator import build_synthetic_response_events
from litellm.types.llms.openai import (
    GenericEvent,
    ResponseAPIUsage,
    ResponseCompletedEvent,
    ResponsesAPIResponse,
    ResponsesAPIStreamEvents,
)
from pydantic import BaseModel, JsonValue, TypeAdapter

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)

BILLED = primitives._billed_terminal_response  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/streaming_iterator.py#L1490
STAMP = primitives._stamp_responses_usage_cost  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/streaming_iterator.py#L1580
COPY = primitives._logging_copy  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/streaming_iterator.py#L1520
DUMP = primitives._dump_json_safely  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/streaming_iterator.py#L1499


class Spec(TypedDict):
    kind: Literal["value", "null", "raise", "empty"]
    cost: NotRequired[str]
    input_tokens: NotRequired[int]
    output_tokens: NotRequired[int]


class Row(TypedDict):
    kind: NotRequired[Literal["typed", "dict", "other", "value", "completed", "generic"]]
    response: NotRequired[JsonValue]
    chunk_size: NotRequired[int]
    cost: NotRequired[Spec | None]
    estimate: NotRequired[Spec | None]
    event: NotRequired[JsonValue]


class Call(TypedDict):
    operation: Literal["build", "terminal", "stamp", "copy", "dump"]
    rows: list[Row]


Input: TypeAlias = Call
RowType = TypeVar("RowType")


class CostProbe:
    def __init__(self, spec: Spec) -> None:
        cost = spec.get("cost")
        self.answer = None if spec["kind"] != "value" or cost is None else float(cost)
        self.raises = spec["kind"] == "raise"
        self.calls = 0

    def _response_cost_calculator(
        self,
        result: ResponsesAPIResponse,
        cache_hit: bool | None = None,
        litellm_model_name: str | None = None,
        router_model_id: str | None = None,
    ) -> float | None:
        del result, cache_hit, litellm_model_name, router_model_id
        self.calls += 1
        if self.raises:
            raise ValueError("cost lookup failed")
        return self.answer


class EstimateProbe:
    def __init__(self, spec: Spec) -> None:
        self.spec = spec
        self.calls = 0

    def __call__(self) -> ResponseAPIUsage | None:
        self.calls += 1
        if self.spec["kind"] == "empty":
            return None
        input_tokens = self.spec.get("input_tokens")
        output_tokens = self.spec.get("output_tokens")
        if input_tokens is None or output_tokens is None:
            return None
        return ResponseAPIUsage(
            input_tokens=input_tokens,
            output_tokens=output_tokens,
            total_tokens=input_tokens + output_tokens,
        )


def logging_object(spec: Spec | None) -> tuple[CostProbe | None, LiteLLMLoggingObj | None]:
    if spec is None:
        return (None, None)
    probe = CostProbe(spec)
    return (probe, cast(LiteLLMLoggingObj, probe))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/streaming_iterator.py#L1602-L1608


def dumped(model: BaseModel) -> JsonValue:
    return cast(JsonValue, model.model_dump())  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/base.py#L29-L30


def rows_answer(rows: list[RowType], build: Callable[[RowType], JsonValue]) -> JsonValue:
    answers: list[JsonValue] = [build(row) for row in rows]
    return {"rows": answers}


def build_answer(row: Row) -> JsonValue:
    response = ResponsesAPIResponse.model_validate(row["response"])
    probe, logger = logging_object(row["cost"])
    try:
        events = build_synthetic_response_events(
            transformed=response, logging_obj=logger, chunk_size=row["chunk_size"]
        )
    except ValueError as error:
        return {
            "raised": True,
            "message": str(error),
            "events": None,
            "cost_calls": 0 if probe is None else probe.calls,
        }
    return {
        "raised": False,
        "message": None,
        "events": [dumped(event) for event in events],
        "cost_calls": 0 if probe is None else probe.calls,
    }


def terminal_answer(row: Row) -> JsonValue:
    estimate = None if row["estimate"] is None else EstimateProbe(row["estimate"])
    if row["kind"] == "other":
        return {"same": False, "response": None, "estimate_calls": estimate_calls(estimate)}
    if row["kind"] == "dict":
        constructed = BILLED(row["response"], estimate)
        return {
            "same": False,
            "response": None if constructed is None else dumped(constructed),
            "estimate_calls": estimate_calls(estimate),
        }
    response = ResponsesAPIResponse.model_validate(row["response"])
    billed = BILLED(response, estimate)
    return {
        "same": billed is response,
        "response": None if billed is None else dumped(billed),
        "estimate_calls": estimate_calls(estimate),
    }


def estimate_calls(estimate: EstimateProbe | None) -> int:
    return 0 if estimate is None else estimate.calls


def stamp_answer(row: Row) -> JsonValue:
    probe, logger = logging_object(row["cost"])
    constructed = BILLED(row["response"], None)
    if constructed is None:
        return {"response": None, "cost_calls": 0 if probe is None else probe.calls}
    STAMP(constructed, logger)
    return {"response": dumped(constructed), "cost_calls": 0 if probe is None else probe.calls}


def copy_answer(row: Row) -> JsonValue:
    if row["kind"] == "value":
        event = row["event"]
        copied = COPY(event)
        return {
            "same": copied is event,
            "original": event,
            "copy": cast(JsonValue, copied),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/streaming_iterator.py#L1490
        }
    if row["kind"] == "completed":
        response = ResponsesAPIResponse.model_validate(row["response"])
        completed = ResponseCompletedEvent(
            type=ResponsesAPIStreamEvents.RESPONSE_COMPLETED, response=response
        )
        copied = COPY(completed)
        finished = cast(ResponseCompletedEvent, copied)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/streaming_iterator.py#L1490
        usage = finished.response.usage
        if usage is not None:
            usage.cost = 0.25
        return {"same": copied is completed, "original": dumped(completed), "copy": dumped(finished)}
    generic = GenericEvent.model_validate(row["event"])
    copied = COPY(generic)
    detached = cast(GenericEvent, copied)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/streaming_iterator.py#L1490
    return {"same": copied is generic, "original": dumped(generic), "copy": dumped(detached)}


def dump_answer(row: Row) -> JsonValue:
    dumped_json = DUMP(ResponsesAPIResponse.model_validate(row["response"]))
    if dumped_json is None:
        return {"dumped": None}
    return {"dumped": JSON.validate_json(dumped_json)}


def run(input_value: Input) -> JsonValue:
    if input_value["operation"] == "build":
        return rows_answer(input_value["rows"], build_answer)
    if input_value["operation"] == "terminal":
        return rows_answer(input_value["rows"], terminal_answer)
    if input_value["operation"] == "stamp":
        return rows_answer(input_value["rows"], stamp_answer)
    if input_value["operation"] == "copy":
        return rows_answer(input_value["rows"], copy_answer)
    return rows_answer(input_value["rows"], dump_answer)
