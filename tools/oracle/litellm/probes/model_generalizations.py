import logging
from collections.abc import Mapping
from typing import Annotated, Final, Literal, Protocol

from litellm.litellm_core_utils import fallback_generalizations as engine
from litellm.litellm_core_utils.get_llm_provider_logic import get_llm_provider
from pydantic import Field, JsonValue, TypeAdapter
from typing_extensions import override

from probes.protocol import Operation, ProbeError, Strict
from probes.tag_matchers import Collector

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true, reportMissingTypeStubs=false


# LiteLLM annotates the installer with a bare list and the capability lookup with a bare dict,
# so strict mode reads both as partially unknown; this protocol states the JSON contract.
class RuleEngine(Protocol):
    @staticmethod
    def set_fallback_generalizations(rules: list[JsonValue] | None) -> None: ...

    @staticmethod
    def match_routing_generalization(model: str) -> str | None: ...

    @staticmethod
    def match_capability_generalizations(model: str) -> dict[str, JsonValue] | None: ...

    @staticmethod
    def match_fill_missing_generalizations(model: str, provider: str) -> Mapping[str, object] | None: ...


ENGINE: RuleEngine = engine

EMPTY_DOCUMENT: Final = "[]"
INFO: Final = TypeAdapter(JsonValue)
RULE_LIST: Final = TypeAdapter(list[JsonValue])


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
            return RoutingAnswer(provider=ENGINE.match_routing_generalization(query.model), warnings=[])
        case CapabilitiesQuery():
            return CapabilitiesAnswer(
                model_info=INFO.validate_python(ENGINE.match_capability_generalizations(query.model)),
                warnings=[],
            )
        case FillMissingQuery():
            return FillMissingAnswer(
                model_info=INFO.validate_python(
                    ENGINE.match_fill_missing_generalizations(query.model, query.provider)
                ),
                warnings=[],
            )
        case ProviderQuery():
            _, provider, _, _ = get_llm_provider(query.model, query.exact)
            return ProviderAnswer(provider=provider, warnings=[])


def parse_rules(payload: ParsePayload) -> ValidParse:
    if payload.bytes < len(EMPTY_DOCUMENT):
        raise ProbeError("invalid_request", "the parse payload is shorter than an empty rule list")
    document = EMPTY_DOCUMENT + " " * (payload.bytes - len(EMPTY_DOCUMENT))
    rules = RULE_LIST.validate_json(document)
    ENGINE.set_fallback_generalizations(rules)
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
            ENGINE.set_fallback_generalizations(self.rules)
        finally:
            logger.removeHandler(collector)
        return Output(
            warnings=[],
            engine_warnings=collector.messages,
            results=[answer_query(query) for query in self.queries],
            parse=None if self.parse is None else parse_rules(self.parse),
        )
