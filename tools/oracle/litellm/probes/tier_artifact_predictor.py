import asyncio
from typing import Final, Literal, NotRequired

from litellm.router import Router
from litellm.router_strategy.complexity_router.complexity_router import ComplexityRouter
from litellm.router_strategy.complexity_router.config import TIER_SEVERITY_ORDER, ComplexityTier
from litellm.router_strategy.complexity_router.tier_predictor import (
    TierSuccessPredictor,
    TrainedTierArtifact,
    resolve_tier_artifact,
    similarity_cohort,
)
from litellm.types.router import RequestType
from pydantic import TypeAdapter, ValidationError
from typing_extensions import TypedDict, override

from probes.protocol import Operation, ProbeError, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true, reportMissingTypeStubs=false

_ARTIFACT: Final = TypeAdapter(TrainedTierArtifact)
_TIER: Final = TypeAdapter(ComplexityTier)
_REQUEST_TYPE: Final = TypeAdapter(RequestType)
_TIERS: Final = (1, 2, 3, 4)
_MAX_EXPANDED_BYTES: Final = 2_097_152


class StatisticInput(TypedDict):
    tier: int
    successes: float
    observations: float


class DomainStatisticInput(StatisticInput):
    request_type: str


class CohortStatisticInput(StatisticInput):
    cohort: str


class DatasetInput(TypedDict):
    name: str
    url: str
    license: str
    rows: int
    success_definition: NotRequired[str]


class ArtifactInput(TypedDict):
    schema_version: NotRequired[int]
    global_statistics: list[StatisticInput]
    domain_statistics: NotRequired[list[DomainStatisticInput]]
    cohort_statistics: NotRequired[list[CohortStatisticInput]]
    domain_prior_mass: NotRequired[float]
    cohort_prior_mass: NotRequired[float]
    routing_threshold: NotRequired[float]
    datasets: NotRequired[list[DatasetInput]]
    success_definition: NotRequired[str]
    split_method: NotRequired[str]


class RouterConfig(TypedDict):
    classifier_type: Literal["heuristic_v2"]
    heuristic_v2_artifact: ArtifactInput | str
    heuristic_v2_success_threshold: float | None


class ArtifactBytes(Strict):
    kind: Literal["text", "padded"]
    content: str
    pad: str | None = None
    pad_len: int | None = None


class Output(Strict):
    cohort: str | None = None
    status: str | None = None
    tier: str | None = None
    score: float | None = None
    signals: list[str] | None = None
    cause: str | None = None
    probabilities: list[float] | None = None
    threshold: float | None = None
    predicted_tier: str | None = None
    request_type: str | None = None


class TierArtifactPredictor(Operation):
    function: Literal["similarity_cohort", "predict", "classify", "startup", "parse_artifact"]
    artifact_name: str | None = None
    artifact: ArtifactInput | None = None
    threshold: str | None = None
    prompt: str = ""
    request_type: str | None = None
    artifact_bytes: ArtifactBytes | None = None

    @override
    def run(self) -> Output:
        if self.function == "similarity_cohort":
            return self._cohort()
        if self.function == "predict":
            return self._predict()
        if self.function == "classify":
            return self._classify()
        if self.function == "startup":
            return Output(status=self._startup())
        return self._parse()

    def _resolved_artifact(self) -> TrainedTierArtifact:
        if self.artifact is not None:
            return _ARTIFACT.validate_python(self.artifact)
        return resolve_tier_artifact(
            self.artifact_name if self.artifact_name is not None else "ultrafeedback"
        )

    def _threshold(self) -> float | None:
        return None if self.threshold is None else float(self.threshold)

    def _request_type(self) -> RequestType:
        if self.request_type is None:
            raise ProbeError("invalid_request", "request_type is required for this function")
        return _REQUEST_TYPE.validate_python(self.request_type)

    def _router_config(self) -> RouterConfig:
        artifact: str | ArtifactInput = self.artifact_name if self.artifact_name is not None else "ultrafeedback"
        if self.artifact is not None:
            artifact = self.artifact
        return {
            "classifier_type": "heuristic_v2",
            "heuristic_v2_artifact": artifact,
            "heuristic_v2_success_threshold": self._threshold(),
        }

    def _cohort(self) -> Output:
        return Output(cohort=similarity_cohort(self.prompt, self._request_type()))

    def _predict(self) -> Output:
        predictor = TierSuccessPredictor(self._resolved_artifact(), routing_threshold=self._threshold())
        prediction = predictor.predict(self.prompt, self._request_type())
        return Output(
            probabilities=[prediction.probabilities[tier] for tier in _TIERS],
            threshold=predictor.routing_threshold,
            predicted_tier=TIER_SEVERITY_ORDER[prediction.required_tier - 1].value,
        )

    def _classify(self) -> Output:
        router = ComplexityRouter(
            model_name="probe",
            litellm_router_instance=Router(),
            complexity_router_config=self._router_config(),
        )
        outcome = asyncio.run(router.aclassify(self.prompt))
        forecast = outcome.heuristic_v2_forecast
        if forecast is None:
            raise ProbeError("invalid_request", "the heuristic_v2 outcome carries no forecast")
        return Output(
            tier=_TIER.validate_python(outcome.tier).name,
            score=outcome.score,
            signals=list(outcome.signals),
            cause=outcome.cause,
            probabilities=[forecast["probabilities"][tier.value] for tier in TIER_SEVERITY_ORDER],
            threshold=forecast["threshold"],
            predicted_tier=forecast["predicted_tier"],
            request_type=forecast["request_type"],
        )

    def _startup(self) -> str:
        try:
            _ = ComplexityRouter(
                model_name="probe",
                litellm_router_instance=Router(),
                complexity_router_config=self._router_config(),
            )
        except ValueError:
            return "rejected"
        return "accepted"

    def _parse(self) -> Output:
        try:
            artifact = TrainedTierArtifact.model_validate_json(self._bytes())
        except ValidationError:
            return Output(status="rejected")
        return Output(status="accepted", threshold=artifact.routing_threshold)

    def _bytes(self) -> bytes:
        if self.artifact_bytes is None:
            raise ProbeError("invalid_request", "artifact_bytes is required for parse_artifact")
        content = self.artifact_bytes.content.encode()
        if self.artifact_bytes.kind == "text":
            if len(content) > _MAX_EXPANDED_BYTES:
                raise ProbeError("invalid_request", f"artifact over {_MAX_EXPANDED_BYTES} bytes")
            return content
        pad = self.artifact_bytes.pad
        pad_len = self.artifact_bytes.pad_len
        if pad is None or pad_len is None:
            raise ProbeError("invalid_request", "a padded artifact needs pad and pad_len")
        if pad_len < 0:
            raise ProbeError("invalid_request", "pad_len cannot be negative")
        if len(content) + len(pad.encode()) * pad_len > _MAX_EXPANDED_BYTES:
            raise ProbeError("invalid_request", f"artifact over {_MAX_EXPANDED_BYTES} bytes")
        return (self.artifact_bytes.content + pad * pad_len).encode()
