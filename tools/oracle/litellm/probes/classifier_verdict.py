import json
from typing import Annotated, Final, Literal, TypeVar, TypedDict

from litellm.litellm_core_utils.internal_call_metadata import effective_turn_off_message_logging
from litellm.litellm_core_utils.redact_messages import should_redact_message_logging
from litellm.llms.base_llm.base_utils import type_to_response_format_param
from litellm.router_strategy.complexity_router.capability_classifier import extract_classifier_json
from litellm.router_strategy.complexity_router.classification_rubrics import calibration_examples_section
from litellm.router_strategy.complexity_router.complexity_router import (
    TIER_SEVERITY_ORDER_LABELED,
    TierClassification,
    built_in_tier_classification_prompt,
    classification_system_prompt,
    custom_tier_classification_prompt,
)
from litellm.router_strategy.complexity_router.config import (
    ClassificationRubric,
    ClassifierLLMConfig,
    ComplexityRouterConfig,
    ComplexityTier,
    TierDefinition,
)
from pydantic import Field, JsonValue, TypeAdapter, ValidationError
from typing_extensions import override

from probes.protocol import Operation, ProbeError, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true, reportMissingTypeStubs=false

Mode = Literal[
    "heuristic", "heuristic_v2", "llm", "capability", "llm_v2", "custom", "heuristic_first", "hybrid", "jev"
]
Rubric = Literal["legacy", "agentic", "chat", "business"]
Function = Literal[
    "system_prompt", "built_in_prompt", "custom_prompt", "calibration_examples",
    "response_format", "resolve_label", "parse_reply", "logging_flag",
]

MAX_PADDED_BYTES: Final = 2_097_152
_CONFIG: Final = TypeAdapter(dict[str, JsonValue])
T = TypeVar("T")


class LabeledTier(Strict):
    tier: Literal["NON_REASONING", "SIMPLE", "MEDIUM", "COMPLEX", "REASONING"]
    label: str


class DefinitionInput(Strict):
    name: str
    description: str | None = None


# the aliases read the case's betterlite key names into LiteLLM's config field names
class TierConfigInput(Strict):
    tier_definitions: list[DefinitionInput] | None = None
    enable_non_reasoning_tier: bool = Field(default=False, validation_alias="non_reasoning_tier")
    tier_labels: dict[str, str] | None = None
    tiers: dict[str, str] | None = Field(default=None, validation_alias="tier_models")
    classifier_type: Mode | None = None
    fallback_tier: str | None = None
    classifier_llm_config: ClassifierLLMConfig | None = None


class TextReply(Strict):
    kind: Literal["text"]
    content: str


class PaddedReply(Strict):
    kind: Literal["padded"]
    content: str
    pad: str
    pad_len: int


Reply = Annotated[TextReply | PaddedReply, Field(discriminator="kind")]


class SelectedParse(Strict):
    kind: Literal["selected"]
    label: str
    tier: str


class RejectedParse(Strict):
    kind: Literal["rejected"]
    reason: Literal["schema", "unknown_tier", "too_large"]
    withheld: bool


ParseOutcome = Annotated[SelectedParse | RejectedParse, Field(discriminator="kind")]


class Output(Strict):
    prompt: str | None = None
    examples: str | None = None
    resolved: str | None = None
    response_format: str | None = None
    parse: ParseOutcome | None = None
    logging_flag: bool | None = None


class ClassifierVerdict(Operation):
    function: Function
    rubric: Rubric | None = None
    labeled_tiers: list[LabeledTier] | None = None
    window: int | None = None
    prompt: str | None = None
    examples: str | None = None
    definitions: list[DefinitionInput] | None = None
    tier_config: TierConfigInput | None = None
    reply: Reply | None = None
    request_kwargs: dict[str, JsonValue] | None = None
    label: str | None = None

    @override
    def run(self) -> Output:
        output = Output()
        match self.function:
            case "system_prompt":
                output = Output(prompt=system_prompt(self))
            case "built_in_prompt":
                output = Output(prompt=built_in_prompt(self))
            case "custom_prompt":
                output = Output(prompt=custom_prompt(self))
            case "calibration_examples":
                output = Output(examples=calibration(self))
            case "response_format":
                output = Output(response_format=rendered_format())
            case "resolve_label":
                config = router_config(self.tier_config)
                label = require(self.label, "label")
                output = Output(resolved=resolved_name(config.resolve_classified_tier(label)))
            case "parse_reply":
                config = router_config(self.tier_config)
                content = reply_content(require(self.reply, "reply"))
                output = Output(parse=parse_outcome(config, content, self.request_kwargs))
            case "logging_flag":
                kwargs: dict[str, JsonValue] = {} if self.request_kwargs is None else self.request_kwargs
                output = Output(logging_flag=effective_turn_off_message_logging(kwargs))
        return output


def require(value: T | None, name: str) -> T:
    if value is None:
        raise ProbeError("invalid_request", f"{name} is required for this function")
    return value


def router_config(configured: TierConfigInput | None) -> ComplexityRouterConfig:
    if configured is None:
        return ComplexityRouterConfig()
    values = _CONFIG.validate_json(configured.model_dump_json(exclude_none=True))
    return ComplexityRouterConfig.model_validate(values)


def labeled_pairs(items: list[LabeledTier] | None) -> list[tuple[ComplexityTier, str]]:
    if items is None:
        return list(TIER_SEVERITY_ORDER_LABELED)
    return [(ComplexityTier(item.tier), item.label) for item in items]


def resolved_name(tier: ComplexityTier | str | None) -> str | None:
    match tier:
        case ComplexityTier():
            return tier.value
        case str():
            return tier
        case None:
            return None


def reply_content(reply: Reply) -> str:
    match reply:
        case TextReply():
            return reply.content
        case PaddedReply():
            if not reply.pad or not reply.pad.isascii() or reply.pad_len < 0 or reply.pad_len > MAX_PADDED_BYTES:
                raise ProbeError("invalid_request", "a padded reply needs an ASCII pad of at most 2097152 bytes")
            return reply.content + reply.pad * reply.pad_len


def reply_private(request_kwargs: dict[str, JsonValue] | None) -> bool:
    kwargs: dict[str, JsonValue] = {} if request_kwargs is None else request_kwargs
    try:
        flag = effective_turn_off_message_logging(kwargs)
        dynamic: dict[str, bool] = {} if flag is None else {"turn_off_message_logging": flag}
        return should_redact_message_logging({"litellm_params": kwargs, "standard_callback_dynamic_params": dynamic})
    except AttributeError:
        return True


def parse_outcome(
    config: ComplexityRouterConfig,
    content: str,
    request_kwargs: dict[str, JsonValue] | None,
) -> SelectedParse | RejectedParse:
    try:
        verdict = TierClassification.model_validate_json(extract_classifier_json(content))
    except ValidationError:
        return RejectedParse(kind="rejected", reason="schema", withheld=reply_private(request_kwargs))
    tier = resolved_name(config.resolve_classified_tier(verdict.tier))
    if tier is None:
        return RejectedParse(kind="rejected", reason="unknown_tier", withheld=False)
    return SelectedParse(kind="selected", label=verdict.tier, tier=tier)


def rendered_format() -> str:
    body: JsonValue = type_to_response_format_param(TierClassification)
    if body is None:
        raise ProbeError("invalid_request", "TierClassification has no response format")
    return json.dumps(body, separators=(",", ":"))


def system_prompt(operation: ClassifierVerdict) -> str:
    return classification_system_prompt(
        require(operation.window, "window"),
        operation.prompt,
        labeled_pairs(operation.labeled_tiers),
        None if operation.rubric is None else ClassificationRubric(operation.rubric),
    )


def built_in_prompt(operation: ClassifierVerdict) -> str:
    return built_in_tier_classification_prompt(
        operation.prompt,
        require(operation.window, "window"),
        labeled_pairs(operation.labeled_tiers),
        None if operation.rubric is None else ClassificationRubric(operation.rubric),
        classification_examples=operation.examples,
    )


def custom_prompt(operation: ClassifierVerdict) -> str:
    definitions = require(operation.definitions, "definitions")
    return custom_tier_classification_prompt(
        tuple(TierDefinition(name=item.name, description=item.description) for item in definitions),
        operation.prompt,
        require(operation.window, "window"),
        classification_examples=operation.examples,
    )


def calibration(operation: ClassifierVerdict) -> str:
    return calibration_examples_section(
        ClassificationRubric(require(operation.rubric, "rubric")),
        labeled_pairs(operation.labeled_tiers),
    )
