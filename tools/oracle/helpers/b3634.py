from datetime import UTC, datetime
from decimal import Decimal
from typing import Literal, TypedDict, cast

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.proxy.pass_through_endpoints.llm_provider_handlers import (
    anthropic_passthrough_logging_handler as handler_module,
)
from litellm.proxy.pass_through_endpoints.success_handler import PassThroughEndpointLogging
from litellm.types.passthrough_endpoints.pass_through_endpoints import EndpointType
from litellm.types.utils import ModelResponse, TextCompletionResponse
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter, ValidationError

litellm.suppress_debug_info = True

Handler = handler_module.AnthropicPassthroughLoggingHandler
BATCH_RESPONSE_ID = "b3634-batch-id"
START: datetime = datetime(2026, 1, 1, tzinfo=UTC)


class Input(TypedDict):
    operation: Literal["handler", "chunks"]
    status_code: int
    url_route: str
    body: str
    request_body: dict[str, JsonValue]
    litellm_params: dict[str, JsonValue]
    passthrough_logging_payload: dict[str, JsonValue]
    details: dict[str, JsonValue]
    lines: list[str]


class UsageRow(TypedDict, total=False):
    prompt_tokens: int | None
    completion_tokens: int | None
    total_tokens: int | None
    speed: str | None


class MessageRow(TypedDict, total=False):
    role: str | None
    content: str | None


class ChoiceRow(TypedDict, total=False):
    finish_reason: str | None
    message: MessageRow | None


class ResultDump(TypedDict, total=False):
    id: str | None
    object: str | None
    model: str | None
    choices: list[ChoiceRow]
    usage: UsageRow | None


class StoredResponse(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    id: str | None = None
    model: str | None = None


class DetailsView(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    model: str | None = None
    response_cost: float | None = None
    custom_llm_provider: str | None = None
    batch_id: str | None = None
    complete_streaming_response: StoredResponse | None = None


class LoggingView(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    model_call_details: DetailsView


class KwargsView(BaseModel):
    response_cost: float | None = None
    model: str | None = None
    litellm_params: JsonValue = None
    batch_id: str | None = None
    unified_object_id: str | None = None
    batch_job_state: str | None = None


class OutcomeView(BaseModel):
    result: ModelResponse | TextCompletionResponse | None
    kwargs: dict[str, JsonValue]


class PricedCall(BaseModel):
    model: str | None
    custom_llm_provider: str | None
    custom_pricing: bool | None
    router_model_id: str | None
    speed: str | None


BODY: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])
JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
RESULT: TypeAdapter[ResultDump] = TypeAdapter(ResultDump)
OUTCOME: TypeAdapter[OutcomeView] = TypeAdapter(OutcomeView)
LOGGING: TypeAdapter[LoggingView] = TypeAdapter(LoggingView)


class Recorder:
    def __init__(self) -> None:
        self.calls: list[PricedCall] = []

    def __call__(
        self,
        completion_response: ModelResponse | TextCompletionResponse,
        *,
        model: str | None = None,
        custom_llm_provider: str | None = None,
        custom_pricing: bool | None = None,
        router_model_id: str | None = None,
    ) -> float:
        usage = RESULT.validate_json(completion_response.model_dump_json()).get("usage")
        self.calls.append(
            PricedCall(
                model=model,
                custom_llm_provider=custom_llm_provider,
                custom_pricing=custom_pricing,
                router_model_id=router_model_id,
                speed=usage.get("speed") if usage is not None else None,
            )
        )
        return 1.25


def cost_text(cost: float | None) -> JsonValue:
    if cost is None:
        return None
    return format(Decimal(repr(cost)).normalize(), "f")


def result_value(response: ModelResponse | TextCompletionResponse | None) -> JsonValue:
    if response is None:
        return None
    dump = RESULT.validate_json(response.model_dump_json())
    batch = dump.get("object") == "batch"
    usage: JsonValue = None
    raw_usage = dump.get("usage")
    if raw_usage is not None and not batch:
        usage = JSON.validate_python(raw_usage)
    choices: list[JsonValue] = []
    for row in dump.get("choices", []):
        message = row.get("message")
        choices.append(
            {
                "finish_reason": row.get("finish_reason"),
                "role": None if message is None else message.get("role"),
                "content": None if message is None else message.get("content"),
            }
        )
    return {
        "id": BATCH_RESPONSE_ID if batch else dump.get("id"),
        "object": dump.get("object"),
        "model": dump.get("model"),
        "choices": choices,
        "usage": usage,
    }


def kwargs_value(kwargs: dict[str, JsonValue]) -> JsonValue:
    view = KwargsView.model_validate(kwargs)
    return {
        "response_cost": cost_text(view.response_cost),
        "model": view.model,
        "litellm_params": view.litellm_params,
        "batch_id": view.batch_id,
        "unified_object_id": view.unified_object_id,
        "batch_job_state": view.batch_job_state,
    }


def details_value(logging_obj: Logging) -> JsonValue:
    details = LOGGING.validate_python(logging_obj).model_call_details
    stored: JsonValue = None
    if details.complete_streaming_response is not None:
        stored = JSON.validate_json(details.complete_streaming_response.model_dump_json())
    return {
        "model": details.model,
        "response_cost": cost_text(details.response_cost),
        "custom_llm_provider": details.custom_llm_provider,
        "batch_id": details.batch_id,
        "complete_streaming_response": stored,
    }


def response_body(body: str) -> dict[str, JsonValue]:
    try:
        return BODY.validate_json(body)
    except ValidationError:
        return {}


def handled(input_value: Input, logging_obj: Logging) -> OutcomeView:
    if input_value["operation"] == "handler":
        return OUTCOME.validate_python(Handler.anthropic_passthrough_handler(  # type: ignore[misc]  # pyright: ignore[reportUnknownMemberType, reportUnknownArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/pass_through_endpoints/llm_provider_handlers/anthropic_passthrough_logging_handler.py#L53-L65
            httpx_response=httpx.Response(
                input_value["status_code"], content=input_value["body"].encode()
            ),
            response_body=response_body(input_value["body"]),
            logging_obj=logging_obj,
            url_route=input_value["url_route"],
            result=input_value["body"],
            start_time=START,
            end_time=START,
            cache_hit=False,
            request_body=input_value["request_body"],
            litellm_params=input_value["litellm_params"],
            passthrough_logging_payload=input_value["passthrough_logging_payload"],
        ))
    collected = Handler._handle_logging_anthropic_collected_chunks(  # type: ignore[misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType, reportUnknownArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/pass_through_endpoints/llm_provider_handlers/anthropic_passthrough_logging_handler.py#L468-L478
        litellm_logging_obj=logging_obj,
        passthrough_success_handler_obj=PassThroughEndpointLogging(),
        url_route=input_value["url_route"],
        request_body=input_value["request_body"],
        endpoint_type=EndpointType.ANTHROPIC,
        start_time=START,
        all_chunks=input_value["lines"],
        end_time=START,
    )
    result = cast("ModelResponse | TextCompletionResponse | None", collected["result"])  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L5065
    kwargs = cast("dict[str, JsonValue]", collected["kwargs"])  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L5066
    return OutcomeView(result=result, kwargs=kwargs)


def run(input_value: Input) -> dict[str, JsonValue]:
    logging_obj = Logging(
        model="",
        messages=None,
        stream=False,
        call_type="pass_through_endpoint",
        start_time=START,
        litellm_call_id="b3634-call",
        function_id="b3634",
    )
    logging_obj.model_call_details.update(input_value["details"])  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/litellm_logging.py#L707-L714
    recorder = Recorder()
    litellm.completion_cost = recorder  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/cost_calculator.py#L1332
    outcome = handled(input_value, logging_obj)
    cost: JsonValue = None
    if outcome.result is not None and recorder.calls:
        cost = JSON.validate_json(recorder.calls[-1].model_dump_json())
    return {
        "result": result_value(outcome.result),
        "kwargs": kwargs_value(outcome.kwargs),
        "details": details_value(logging_obj),
        "cost": cost,
    }
