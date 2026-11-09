import json
import logging
from collections.abc import Callable, Sequence
from typing import Annotated, Final, Literal, cast

from litellm._logging import verbose_logger
from litellm.router_strategy import tag_based_routing
from pydantic import Field
from typing_extensions import override

from probes.protocol import Operation, ProbeError, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true, reportMissingTypeStubs=false

MAX_TAG_MATCH_BYTES: Final = 1024 * 1024
EMPTY_DOCUMENT: Final = '{"deployments":[]}'
INVALID_PATTERN_PREFIX: Final = "tag_regex: invalid pattern "

StripPrefix = Callable[[Sequence[str], str], tuple[tuple[str, ...], frozenset[str]]]
SplitTags = Callable[[Sequence[str]], tuple[tuple[str, ...], list[str], tuple[str, ...]]]
ExcludeDeployments = Callable[[Sequence[dict[str, object]], frozenset[str]], list[dict[str, object]]]
RequireAllTags = Callable[[Sequence[dict[str, object]], frozenset[str]], tuple[dict[str, object], ...]]
DefaultPool = Callable[[Sequence[dict[str, object]]], tuple[dict[str, object], ...]]
KnownTags = Callable[[Sequence[dict[str, object]]], frozenset[str]]
MatchDeployment = Callable[[dict[str, object], list[str], list[str], bool], dict[str, str] | None]
UnknownHides = Callable[[Sequence[dict[str, object]], frozenset[str], frozenset[str], frozenset[str]], bool]
ValidRegex = Callable[[Sequence[str], Sequence[str]], str | None]


class DeploymentInput(Strict):
    id: str
    tags: list[str] | None = None
    tag_regex: list[str] | None = None


class ParsePayload(Strict):
    bytes: int = Field(ge=0)


class WarningOutput(Strict):
    kind: Literal["invalid_pattern"] = "invalid_pattern"
    pattern: str


class MatchOutput(Strict):
    id: str
    matched_via: str
    matched_value: str


class TagsOutput(Strict):
    rewritten: list[str]
    required: list[str]
    positive: list[str]
    excluded: list[str]
    routing_confirmed: list[str]
    inherited_required: list[str] | None
    inherited_excluded: list[str] | None


class ValidParse(Strict):
    kind: Literal["valid"] = "valid"
    deployments: int


class InvalidParse(Strict):
    kind: Literal["invalid"] = "invalid"
    bytes: int
    limit: int


ParseOutput = Annotated[ValidParse | InvalidParse, Field(discriminator="kind")]


class Output(Strict):
    tags: TagsOutput
    allowed: list[str]
    candidates: list[str]
    matches: list[MatchOutput]
    defaults: list[str]
    known_tags: list[str]
    unknown_required_hides_answer: bool
    warnings: list[WarningOutput]
    parse: ParseOutput | None


class Collector(logging.Handler):
    def __init__(self) -> None:
        super().__init__()
        self.messages: list[str] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        self.messages.append(record.getMessage())


def deployment_dict(deployment: DeploymentInput) -> dict[str, object]:
    params: dict[str, object] = {}
    if deployment.tags is not None:
        params["tags"] = deployment.tags
    if deployment.tag_regex is not None:
        params["tag_regex"] = deployment.tag_regex
    return {"model_name": deployment.id, "litellm_params": params, "model_info": {}}


def compile_warnings(deployments: list[DeploymentInput], valid_regex: ValidRegex) -> list[WarningOutput]:
    warnings: list[WarningOutput] = []
    for deployment in deployments:
        patterns = deployment.tag_regex or []
        if not patterns:
            continue
        collector = Collector()
        verbose_logger.addHandler(collector)
        try:
            _ = valid_regex(patterns, [])
        finally:
            verbose_logger.removeHandler(collector)
        for pattern in patterns:
            prefix = INVALID_PATTERN_PREFIX + repr(pattern)
            if any(message.startswith(prefix) for message in collector.messages):
                warnings.append(WarningOutput(pattern=pattern))
    return warnings


def inherited_sets(
    tags: list[str] | None,
    prefix: str,
    strip_prefix: StripPrefix,
    split_tags: SplitTags,
) -> tuple[list[str] | None, list[str] | None]:
    if tags is None:
        return None, None
    rewritten, _ = strip_prefix(tags, prefix)
    required, _, excluded = split_tags(rewritten)
    return sorted(required), sorted(excluded)


def deployment_names(pool: Sequence[dict[str, object]], names: dict[int, str]) -> list[str]:
    return [names[id(deployment)] for deployment in pool]


def parse_document(payload: ParsePayload) -> ValidParse:
    if payload.bytes < len(EMPTY_DOCUMENT):
        raise ProbeError("invalid_request", "the parse payload is shorter than an empty tag match document")
    if payload.bytes > MAX_TAG_MATCH_BYTES + 1:
        raise ProbeError("invalid_request", "the parse payload length is past the parser bound")
    document = EMPTY_DOCUMENT + " " * (payload.bytes - len(EMPTY_DOCUMENT))
    parsed = cast("object", json.loads(document))
    if not isinstance(parsed, dict):
        raise ProbeError("invalid_request", "the tag match document is not an object")
    deployments: object = cast("dict[str, object]", parsed).get("deployments")
    if not isinstance(deployments, list):
        raise ProbeError("invalid_request", "the tag match document has no deployment list")
    return ValidParse(deployments=len(cast("list[object]", deployments)))


class TagMatchers(Operation):
    deployments: list[DeploymentInput] = Field(default_factory=list)
    tags: list[str] = Field(default_factory=list)
    inherited_tags: list[str] | None = None
    header_strings: list[str] = Field(default_factory=list)
    match_any: bool = True
    routing_prefix: str = ""
    parse: ParsePayload | None = None

    def match_outputs(
        self,
        candidates: Sequence[dict[str, object]],
        names: dict[int, str],
        positive: list[str],
        match_deployment: MatchDeployment,
    ) -> list[MatchOutput]:
        outputs: list[MatchOutput] = []
        for deployment in candidates:
            matched = match_deployment(deployment, positive, self.header_strings, self.match_any)
            if matched is None:
                continue
            outputs.append(
                MatchOutput(
                    id=names[id(deployment)],
                    matched_via=matched["matched_via"],
                    matched_value=matched["matched_value"],
                )
            )
        return outputs

    @override
    def run(self) -> Output:
        strip_prefix = cast("StripPrefix", tag_based_routing._strip_routing_prefix)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        split_tags = cast("SplitTags", tag_based_routing._split_tags)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        exclude_deployments = cast("ExcludeDeployments", tag_based_routing._exclude_deployments)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        require_all_tags = cast("RequireAllTags", tag_based_routing._require_all_tags)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        default_pool = cast("DefaultPool", tag_based_routing._default_tagged_pool)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        known_tags = cast("KnownTags", tag_based_routing._known_tag_values)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        match_deployment = cast("MatchDeployment", tag_based_routing._match_deployment)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        unknown_hides = cast("UnknownHides", tag_based_routing._unknown_required_tag_hides_an_answer)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        valid_regex = cast("ValidRegex", tag_based_routing._is_valid_deployment_tag_regex)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        deployments = [deployment_dict(deployment) for deployment in self.deployments]
        names = {
            id(deployment): self.deployments[index].id
            for index, deployment in enumerate(deployments)
        }
        rewritten, confirmed = strip_prefix(self.tags, self.routing_prefix)
        required, positive, excluded = split_tags(rewritten)
        required_set = frozenset(required)
        excluded_set = frozenset(excluded)
        allowed = exclude_deployments(deployments, excluded_set)
        candidates = require_all_tags(allowed, required_set)
        inherited_required, inherited_excluded = inherited_sets(
            self.inherited_tags, self.routing_prefix, strip_prefix, split_tags
        )
        return Output(
            tags=TagsOutput(
                rewritten=list(rewritten),
                required=list(required),
                positive=positive,
                excluded=list(excluded),
                routing_confirmed=sorted(confirmed),
                inherited_required=inherited_required,
                inherited_excluded=inherited_excluded,
            ),
            allowed=deployment_names(allowed, names),
            candidates=deployment_names(candidates, names),
            matches=self.match_outputs(candidates, names, positive, match_deployment),
            defaults=deployment_names(default_pool(deployments), names),
            known_tags=sorted(known_tags(deployments)),
            unknown_required_hides_answer=unknown_hides(deployments, excluded_set, required_set, confirmed),
            warnings=compile_warnings(self.deployments, valid_regex),
            parse=None if self.parse is None else parse_document(self.parse),
        )
