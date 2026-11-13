import datetime as dt
import os
from collections.abc import Callable
from decimal import Decimal
from typing import Final, Literal, Protocol, TypedDict

import httpx
import litellm
import litellm.llms.perplexity.cost_calculator as perplexity_cost_calculator
from litellm.litellm_core_utils.get_llm_provider_logic import get_llm_provider
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.perplexity.chat.transformation import PerplexityChatConfig
from litellm.llms.perplexity.cost_calculator import cost_per_token
from litellm.types.utils import ModelInfo, ModelResponse, Usage
from pydantic import JsonValue, TypeAdapter

PRICE_QUANTUM: Final = Decimal("0.000000000001")


class CredentialStep(TypedDict):
    api_base: str | None
    api_key: str | None
    environment: dict[str, str]


class ParamStep(TypedDict):
    model: str
    reasoning: Literal["false", "true", "error"]
    web_search: Literal["false", "true", "error"]


class ReplyStep(TypedDict):
    model: str
    id: str
    body: str
    start_time: str


class PerplexityModelInfo(ModelInfo, total=False):
    search_queries_cost_per_query: float | dict[str, float] | None


class UsageInput(TypedDict, total=False):
    prompt_tokens: int
    completion_tokens: int
    citation_tokens: int
    reasoning_tokens: int
    cost: JsonValue
    prompt_tokens_details: dict[str, int]
    completion_tokens_details: dict[str, int]


class CostStep(TypedDict):
    model: str
    model_info: dict[str, JsonValue]
    usage: UsageInput
    current_time: str


class Input(TypedDict):
    function: Literal["cost", "credentials", "reply", "supported_params"]
    steps: list[JsonValue]


class UrlCitationRow(TypedDict):
    url: str | None
    title: str | None
    start_index: int | None
    end_index: int | None


class AnnotationRow(TypedDict):
    type: str | None
    url_citation: UrlCitationRow


class MessageRow(TypedDict, total=False):
    annotations: list[AnnotationRow] | None


class ChoiceRow(TypedDict, total=False):
    message: MessageRow | None


class UsageRow(TypedDict, total=False):
    prompt_tokens: int | None
    completion_tokens: int | None
    total_tokens: int | None
    citation_tokens: int | None
    prompt_tokens_details: dict[str, int | None] | None


class ReplyRow(TypedDict, total=False):
    usage: UsageRow | None
    choices: list[ChoiceRow]
    citations: JsonValue
    search_results: JsonValue


class SupportsParams(Protocol):
    get_supported_openai_params: Callable[..., list[JsonValue]]


class TransformResponse(Protocol):
    transform_response: Callable[..., ModelResponse]


class CapabilityFixture:
    def __init__(self, capability: str, answer: str) -> None:
        self.capability = capability
        self.answer = answer

    def __call__(self, model: str, custom_llm_provider: str | None = None) -> bool:
        del model, custom_llm_provider
        if self.answer == "true":
            return True
        if self.answer == "false":
            return False
        raise RuntimeError(f"the {self.capability} capability lookup failed")


class ModelInfoLookup:
    def __init__(self, info: PerplexityModelInfo) -> None:
        self.info = info

    def __call__(
        self,
        model: str,
        custom_llm_provider: str | None = None,
        api_base: str | None = None,
        api_key: str | None = None,
    ) -> ModelInfo:
        del model, custom_llm_provider, api_base, api_key
        return self.info


CREDENTIAL_STEPS: Final = TypeAdapter(list[CredentialStep])
PARAM_STEPS: Final = TypeAdapter(list[ParamStep])
REPLY_STEPS: Final = TypeAdapter(list[ReplyStep])
COST_STEPS: Final = TypeAdapter(list[CostStep])
MODEL_INFO: Final = TypeAdapter(PerplexityModelInfo)
REPLY_ROW: Final = TypeAdapter(ReplyRow)


def credential_outcome(step: CredentialStep) -> JsonValue:
    for name in ("PERPLEXITY_API_BASE", "PERPLEXITYAI_API_KEY", "PERPLEXITY_API_KEY"):
        os.environ.pop(name, None)
    for name, value in step["environment"].items():
        os.environ[name] = value
    _, _, key, base = get_llm_provider(
        model="perplexity/sonar", api_base=step["api_base"], api_key=step["api_key"]
    )
    return {"base": base, "key": key}


def param_outcome(config: SupportsParams, step: ParamStep) -> JsonValue:
    litellm.supports_reasoning = CapabilityFixture("reasoning", step["reasoning"])
    litellm.supports_web_search = CapabilityFixture("web search", step["web_search"])
    supported = config.get_supported_openai_params(model=step["model"])
    return {"supported": supported}


def reply_outcome(config: TransformResponse, step: ReplyStep) -> JsonValue:
    logging_obj = Logging(
        model=step["model"],
        messages=[],
        stream=False,
        call_type="completion",
        start_time=dt.datetime.fromisoformat(step["start_time"]),
        litellm_call_id="perplexity-call",
        function_id="perplexity-function",
    )
    result = config.transform_response(
        model=step["model"],
        raw_response=httpx.Response(200, text=step["body"]),
        model_response=ModelResponse(id=step["id"]),
        logging_obj=logging_obj,
        request_data={},
        messages=[],
        optional_params={},
        litellm_params={},
        encoding=None,
    )
    return reply_projection(REPLY_ROW.validate_json(result.model_dump_json()))


def reply_projection(reply: ReplyRow) -> JsonValue:
    usage = reply.get("usage")
    usage_json: JsonValue = None
    if usage is not None:
        details = usage.get("prompt_tokens_details")
        usage_json = {
            "prompt_tokens": usage.get("prompt_tokens"),
            "completion_tokens": usage.get("completion_tokens"),
            "total_tokens": usage.get("total_tokens"),
            "citation_tokens": usage.get("citation_tokens"),
            "web_search_requests": None
            if details is None
            else details.get("web_search_requests"),
        }
    annotations: JsonValue = None
    choices = reply.get("choices")
    message = None if not choices else choices[0].get("message")
    found = None if message is None else message.get("annotations")
    if found is not None:
        rendered: list[JsonValue] = []
        for annotation in found:
            citation = annotation["url_citation"]
            rendered.append(
                {
                    "type": annotation["type"],
                    "url_citation": {
                        "url": citation["url"],
                        "title": citation["title"],
                        "start_index": citation["start_index"],
                        "end_index": citation["end_index"],
                    },
                }
            )
        annotations = rendered
    return {
        "usage": usage_json,
        "annotations": annotations,
        "citations": reply.get("citations"),
        "search_results": reply.get("search_results"),
    }


def build_usage(input_value: UsageInput) -> Usage:
    usage = Usage(
        prompt_tokens=input_value.get("prompt_tokens"),
        completion_tokens=input_value.get("completion_tokens"),
        prompt_tokens_details=input_value.get("prompt_tokens_details"),
        completion_tokens_details=input_value.get("completion_tokens_details"),
    )
    citation_tokens = input_value.get("citation_tokens")
    if citation_tokens is not None:
        usage["citation_tokens"] = citation_tokens
    reasoning_tokens = input_value.get("reasoning_tokens")
    if reasoning_tokens is not None:
        usage["reasoning_tokens"] = reasoning_tokens
    cost = input_value.get("cost")
    if cost is not None:
        usage["cost"] = cost
    return usage


def cost_outcome(step: CostStep) -> JsonValue:
    info = MODEL_INFO.validate_python(step["model_info"])
    usage = build_usage(step["usage"])
    moment = dt.datetime.fromisoformat(step["current_time"])
    previous = perplexity_cost_calculator.get_model_info
    perplexity_cost_calculator.get_model_info = ModelInfoLookup(info)
    try:
        prompt, completion = cost_per_token(
            model=step["model"], usage=usage, current_time=moment
        )
    finally:
        perplexity_cost_calculator.get_model_info = previous
    return {
        "prompt_cost": format(Decimal(repr(prompt)).quantize(PRICE_QUANTUM), "f"),
        "completion_cost": format(Decimal(repr(completion)).quantize(PRICE_QUANTUM), "f"),
    }


def run(input_value: Input) -> JsonValue:
    function = input_value["function"]
    if function == "credentials":
        credential_steps = CREDENTIAL_STEPS.validate_python(input_value["steps"])
        credential_rows: list[JsonValue] = [
            credential_outcome(step) for step in credential_steps
        ]
        return credential_rows
    if function == "supported_params":
        param_steps = PARAM_STEPS.validate_python(input_value["steps"])
        param_rows: list[JsonValue] = [
            param_outcome(PerplexityChatConfig(), step) for step in param_steps
        ]
        return param_rows
    if function == "reply":
        reply_steps = REPLY_STEPS.validate_python(input_value["steps"])
        reply_rows: list[JsonValue] = [
            reply_outcome(PerplexityChatConfig(), step) for step in reply_steps
        ]
        return reply_rows
    cost_steps = COST_STEPS.validate_python(input_value["steps"])
    cost_rows: list[JsonValue] = [cost_outcome(step) for step in cost_steps]
    return cost_rows
