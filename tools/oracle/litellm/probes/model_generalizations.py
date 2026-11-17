import logging
from collections.abc import Callable
from typing import Annotated, Final, Literal

from litellm.litellm_core_utils.fallback_generalizations import (
    match_capability_generalizations,
    match_fill_missing_generalizations,
    match_routing_generalization,
    set_fallback_generalizations,
)
from litellm.litellm_core_utils.get_llm_provider_logic import get_llm_provider
from pydantic import Field, JsonValue, TypeAdapter
from typing_extensions import override

from probes.protocol import Operation, ProbeError, Strict
from probes.tag_matchers import Collector

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true, reportMissingTypeStubs=false

EMPTY_DOCUMENT: Final = "[]"
INFO: Final = TypeAdapter(JsonValue)
RULE_LIST: Final = TypeAdapter(list[JsonValue])

MatchRouting = Callable[[str], str | None]
MatchInfo = Callable[[str], object]
MatchFillMissing = Callable[[str, str], object]
SetRules = Callable[[list[JsonValue] | None], None]
GetProvider = Callable[[str, str | None], tuple[str, str, str | None, str | None]]

MATCH_ROUTING: Final[MatchRouting] = match_routing_generalization
MATCH_CAPABILITIES: Final[MatchInfo] = match_capability_generalizations
MATCH_FILL_MISSING: Final[MatchFillMissing] = match_fill_missing_generalizations
SET_RULES: Final[SetRules] = set_fallback_generalizations
PROVIDER: Final[GetProvider] = get_llm_provider


class ParsePayload(Strict):
    bytes: int = Field(ge=0)


class RoutingQuery(Strict):
    query: Literal["routing"] = "routing"
    model: str


class CapabilitiesQuery(Strict):
    query: Literal["capabilities"] = "capabilities"
    model: str


class FillMissingQuery(Strict):
    query: Literal["fill_missing"] = "fill_missing"
    model: str
    provider: str


class ProviderQuery(Strict):
    query: Literal["provider"] = "provider"
    model: str
    exact: str | None = None


Query = Annotated[RoutingQuery | CapabilitiesQuery | FillMissingQuery | ProviderQuery, Field(discriminator="query")]


class WarningOutput(Strict):
    kind: Literal["invalid_pattern", "unsupported_pattern", "pattern_too_large", "match_limit_exceeded"]
    pattern: str


class RoutingAnswer(Strict):
    kind: Literal["routing"] = "routing"
    provider: str | None
    warnings: list[WarningOutput]


class CapabilitiesAnswer(Strict):
    kind: Literal["capabilities"] = "capabilities"
    model_info: JsonValue
    warnings: list[WarningOutput]


class FillMissingAnswer(Strict):
    kind: Literal["fill_missing"] = "fill_missing"
    model_info: JsonValue
    warnings: list[WarningOutput]


class ProviderAnswer(Strict):
    kind: Literal["provider"] = "provider"
    provider: str | None
    warnings: list[WarningOutput]


Answer = Annotated[RoutingAnswer | CapabilitiesAnswer | FillMissingAnswer | ProviderAnswer, Field(discriminator="kind")]


class ValidParse(Strict):
    kind: Literal["valid"] = "valid"
    rules: int


class InvalidParse(Strict):
    kind: Literal["invalid"] = "invalid"
    bytes: int
    limit: int


ParseOutput = Annotated[ValidParse | InvalidParse, Field(discriminator="kind")]


class Output(Strict):
    warnings: list[WarningOutput]
    engine_warnings: list[str]
    results: list[Answer]
    parse: ParseOutput | None


def answer_query(query: Query) -> Answer:
    match query:
        case RoutingQuery():
            return RoutingAnswer(provider=MATCH_ROUTING(query.model), warnings=[])
        case CapabilitiesQuery():
            matched: object = MATCH_CAPABILITIES(query.model)
            return CapabilitiesAnswer(model_info=INFO.validate_python(matched), warnings=[])
        case FillMissingQuery():
            filled: object = MATCH_FILL_MISSING(query.model, query.provider)
            return FillMissingAnswer(model_info=INFO.validate_python(filled), warnings=[])
        case ProviderQuery():
            _, provider, _, _ = PROVIDER(query.model, query.exact)
            return ProviderAnswer(provider=provider, warnings=[])


def parse_rules(payload: ParsePayload) -> ValidParse:
    if payload.bytes < len(EMPTY_DOCUMENT):
        raise ProbeError("invalid_request", "the parse payload is shorter than an empty rule list")
    document = EMPTY_DOCUMENT + " " * (payload.bytes - len(EMPTY_DOCUMENT))
    rules = RULE_LIST.validate_json(document)
    SET_RULES(rules)
    return ValidParse(rules=len(rules))


class ModelGeneralizations(Operation):
    rules: list[JsonValue] = Field(default_factory=list)
    queries: list[Query] = Field(default_factory=list)
    parse: ParsePayload | None = None

    @override
    def run(self) -> Output:
        collector = Collector()
        logger = logging.getLogger("LiteLLM")
        logger.addHandler(collector)
        try:
            SET_RULES(self.rules)
        finally:
            logger.removeHandler(collector)
        return Output(
            warnings=[],
            engine_warnings=collector.messages,
            results=[answer_query(query) for query in self.queries],
            parse=None if self.parse is None else parse_rules(self.parse),
        )
