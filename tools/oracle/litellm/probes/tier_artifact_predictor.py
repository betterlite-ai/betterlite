import asyncio
from typing import Final, Literal

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
from pydantic import JsonValue, TypeAdapter, ValidationError
from typing_extensions import override

from probes.protocol import Operation, ProbeError, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true, reportMissingTypeStubs=false, reportUnusedCallResult=false

_ARTIFACT: Final = TypeAdapter(TrainedTierArtifact)
_TIER: Final = TypeAdapter(ComplexityTier)
_REQUEST_TYPE: Final = TypeAdapter(RequestType)
_TIERS: Final = (1, 2, 3, 4)


class ArtifactBytes(Strict):
    kind: Literal["text", "padded"]
    content: str
    pad: str | None = None
    pad_len: int | None = None


class Output(Strict):
    cohort: str | None = None
    status: str | None = None
    tier: str | None = None
    score: str | None = None
    signals: list[str] | None = None
    cause: str | None = None
    probabilities: list[str] | None = None
    threshold: str | None = None
    predicted_tier: str | None = None
    request_type: str | None = None


class TierArtifactPredictor(Operation):
    function: Literal["similarity_cohort", "predict", "classify", "startup", "parse_artifact"]
    artifact_name: str | None = None
    artifact: dict[str, JsonValue] | None = None
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

    def _artifact_value(self) -> str | dict[str, JsonValue]:
        if self.artifact is not None:
            return self.artifact
        return self.artifact_name if self.artifact_name is not None else "ultrafeedback"

    def _resolved_artifact(self) -> TrainedTierArtifact:
        if self.artifact is not None:
            return _ARTIFACT.validate_python(self.artifact)
        if self.artifact_name is not None:
            return resolve_tier_artifact(self.artifact_name)
        return resolve_tier_artifact("ultrafeedback")

    def _threshold(self) -> float | None:
        return None if self.threshold is None else float(self.threshold)

    def _request_type(self) -> RequestType:
        if self.request_type is None:
            raise ProbeError("invalid_request", "request_type is required for this function")
        return _REQUEST_TYPE.validate_python(self.request_type)

    def _router_config(self) -> dict[str, object]:
        # the section's keys as the reference's complexity router config reads them
        return {
            "classifier_type": "heuristic_v2",
            "heuristic_v2_artifact": self._artifact_value(),
            "heuristic_v2_success_threshold": self._threshold(),
        }

    def _cohort(self) -> Output:
        return Output(cohort=similarity_cohort(self.prompt, self._request_type()))

    def _predict(self) -> Output:
        predictor = TierSuccessPredictor(self._resolved_artifact(), routing_threshold=self._threshold())
        prediction = predictor.predict(self.prompt, self._request_type())
        return Output(
            probabilities=[f"{prediction.probabilities[tier]:.6f}" for tier in _TIERS],
            threshold=f"{predictor.routing_threshold:.6f}",
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
            score=None if outcome.score is None else f"{outcome.score:.6f}",
            signals=list(outcome.signals),
            cause=outcome.cause,
            probabilities=[f"{forecast['probabilities'][tier.value]:.6f}" for tier in TIER_SEVERITY_ORDER],
            threshold=f"{forecast['threshold']:.6f}",
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
        return Output(status="accepted", threshold=f"{artifact.routing_threshold:.6f}")

    def _bytes(self) -> bytes:
        if self.artifact_bytes is None:
            raise ProbeError("invalid_request", "artifact_bytes is required for parse_artifact")
        if self.artifact_bytes.kind == "text":
            return self.artifact_bytes.content.encode()
        if self.artifact_bytes.pad is None or self.artifact_bytes.pad_len is None:
            raise ProbeError("invalid_request", "a padded artifact needs pad and pad_len")
        return (self.artifact_bytes.content + self.artifact_bytes.pad * self.artifact_bytes.pad_len).encode()
