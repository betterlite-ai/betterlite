import datetime as dt
import os
from collections.abc import Callable
from decimal import Decimal
from typing import Final, Literal, Protocol, TypeAlias, TypedDict, runtime_checkable

import httpx
import litellm
import litellm.llms.perplexity.cost_calculator as perplexity_cost_calculator
from litellm.litellm_core_utils.get_llm_provider_logic import get_llm_provider
from litellm.llms.perplexity.chat.transformation import PerplexityChatConfig
from litellm.llms.perplexity.cost_calculator import cost_per_token
from litellm.types.utils import ModelResponse, Usage
from pydantic import ConfigDict, JsonValue, TypeAdapter

PRICE_QUANTUM: Final = Decimal("0.000000000001")
ARBITRARY_TYPES: Final = ConfigDict(arbitrary_types_allowed=True)
ModelInfoCall: TypeAlias = Callable[
    [str, str | None, str | None, str | None], dict[str, JsonValue]
]


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


class Recorder:
    def post_call(self, **_fields: JsonValue) -> None: ...


@runtime_checkable
class ChatConfig(Protocol):
    get_supported_openai_params: Callable[[str], list[JsonValue]]
    transform_response: Callable[
        [
            str,
            httpx.Response,
            ModelResponse,
            Recorder,
            dict[str, JsonValue],
            list[JsonValue],
            dict[str, JsonValue],
            dict[str, JsonValue],
            None,
        ],
        ModelResponse,
    ]


@runtime_checkable
class CostModule(Protocol):
    get_model_info: ModelInfoCall


class FixedModelInfo:
    def __init__(self, info: dict[str, JsonValue]) -> None:
        self.info = info

    def __call__(
        self,
        model: str,
        custom_llm_provider: str | None = None,
        api_base: str | None = None,
        api_key: str | None = None,
    ) -> dict[str, JsonValue]:
        del model, custom_llm_provider, api_base, api_key
        return self.info


CONFIG: Final[ChatConfig] = TypeAdapter[ChatConfig](
    ChatConfig, config=ARBITRARY_TYPES
).validate_python(PerplexityChatConfig())
COST_CALCULATOR: Final[CostModule] = TypeAdapter[CostModule](
    CostModule, config=ARBITRARY_TYPES
).validate_python(perplexity_cost_calculator)
CREDENTIAL_STEPS: Final = TypeAdapter(list[CredentialStep])
PARAM_STEPS: Final = TypeAdapter(list[ParamStep])
REPLY_STEPS: Final = TypeAdapter(list[ReplyStep])
COST_STEPS: Final = TypeAdapter(list[CostStep])
REPLY_ROW: Final = TypeAdapter(ReplyRow)


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


def credential_outcome(step: CredentialStep) -> JsonValue:
    for name in ("PERPLEXITY_API_BASE", "PERPLEXITYAI_API_KEY", "PERPLEXITY_API_KEY"):
        os.environ.pop(name, None)
    for name, value in step["environment"].items():
        os.environ[name] = value
    _, _, key, base = get_llm_provider(
        model="perplexity/sonar", api_base=step["api_base"], api_key=step["api_key"]
    )
    return {"base": base, "key": key}


def param_outcome(step: ParamStep) -> JsonValue:
    litellm.supports_reasoning = CapabilityFixture("reasoning", step["reasoning"])
    litellm.supports_web_search = CapabilityFixture("web search", step["web_search"])
    supported = CONFIG.get_supported_openai_params(step["model"])
    return {"supported": supported}


def reply_outcome(step: ReplyStep) -> JsonValue:
    result = CONFIG.transform_response(
        step["model"],
        httpx.Response(200, text=step["body"]),
        ModelResponse(id=step["id"]),
        Recorder(),
        {},
        [],
        {},
        {},
        None,
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
    usage = build_usage(step["usage"])
    moment = dt.datetime.fromisoformat(step["current_time"])
    previous = COST_CALCULATOR.get_model_info
    COST_CALCULATOR.get_model_info = FixedModelInfo(step["model_info"])
    try:
        prompt, completion = cost_per_token(
            model=step["model"], usage=usage, current_time=moment
        )
    finally:
        COST_CALCULATOR.get_model_info = previous
    return {
        "prompt_cost": format(Decimal(repr(prompt)).quantize(PRICE_QUANTUM), "f"),
        "completion_cost": format(Decimal(repr(completion)).quantize(PRICE_QUANTUM), "f"),
    }


def run(input_value: Input) -> JsonValue:
    function = input_value["function"]
    if function == "credentials":
        credential_steps = CREDENTIAL_STEPS.validate_python(input_value["steps"])
        return [credential_outcome(step) for step in credential_steps]
    if function == "supported_params":
        param_steps = PARAM_STEPS.validate_python(input_value["steps"])
        return [param_outcome(step) for step in param_steps]
    if function == "reply":
        reply_steps = REPLY_STEPS.validate_python(input_value["steps"])
        return [reply_outcome(step) for step in reply_steps]
    cost_steps = COST_STEPS.validate_python(input_value["steps"])
    return [cost_outcome(step) for step in cost_steps]
