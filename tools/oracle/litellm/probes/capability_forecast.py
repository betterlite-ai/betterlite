import json
from typing import Annotated, Final, Literal, TypeVar

from litellm.router_strategy.complexity_router.capability_classifier import (
    CapabilityClassifierForecast,
    capability_classifier_response_format,
    capability_classifier_system_prompt,
    parse_capability_classifier_verdict,
)
from litellm.router_strategy.complexity_router.config import (
    CapabilityCalibrationConfig,
    CapabilityClassifierConfig,
)
from pydantic import Field, ValidationError
from typing_extensions import override

from probes.protocol import Operation, ProbeError, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true, reportMissingTypeStubs=false

Mode = Literal["json_schema", "json_object"]
Function = Literal["response_format", "system_prompt", "evaluate", "validate_config"]
T = TypeVar("T")

MAX_PADDED_BYTES: Final = 2_097_152


class TextReply(Strict):
    kind: Literal["text"]
    content: str


class PaddedReply(Strict):
    kind: Literal["padded"]
    content: str
    pad: str
    pad_len: int


Reply = Annotated[TextReply | PaddedReply, Field(discriminator="kind")]


class CalibrationInput(Strict):
    version: str
    slope: str
    intercept: str


class ConfigInput(Strict):
    efficient_tier: str
    capable_tier: str
    base_threshold: str
    threshold_step: str | None = None
    max_output_tokens: int | None = None
    calibration: CalibrationInput | None = None
    response_format: Mode | None = None


class RoutedRoute(Strict):
    kind: Literal["routed"]
    tier: str
    threshold: str
    crux: str
    p_solve: str
    meets: bool
    primary_rule: str
    capability_boundary: str
    calibration_version: str | None


class RejectedRoute(Strict):
    kind: Literal["rejected"]


class InvalidConfigRoute(Strict):
    kind: Literal["invalid_config"]


RouteOutcome = Annotated[RoutedRoute | RejectedRoute | InvalidConfigRoute, Field(discriminator="kind")]


class ValidConfig(Strict):
    kind: Literal["valid"]
    efficient_tier: str
    capable_tier: str
    base_threshold: str
    threshold_step: str
    max_output_tokens: int
    response_format: Mode
    calibration_version: str | None


class InvalidConfig(Strict):
    kind: Literal["invalid"]


ConfigOutcome = Annotated[ValidConfig | InvalidConfig, Field(discriminator="kind")]


class Output(Strict):
    response_format: str | None = None
    system_prompt: str | None = None
    route: RouteOutcome | None = None
    config: ConfigOutcome | None = None


def require(value: T | None, name: str) -> T:
    if value is None:
        raise ProbeError("invalid_request", f"{name} is required for this function")
    return value


def parse_float(text: str, name: str) -> float:
    try:
        return float(text)
    except ValueError as error:
        raise ProbeError("invalid_request", f"{name} is not a number: {text}") from error


def reply_content(reply: Reply) -> str:
    if isinstance(reply, TextReply):
        return reply.content
    if not reply.pad or not reply.pad.isascii() or reply.pad_len < 0 or reply.pad_len > MAX_PADDED_BYTES:
        raise ProbeError("invalid_request", "a padded reply needs an ASCII pad and at most 2097152 pad bytes")
    return reply.content + reply.pad * reply.pad_len


def calibration_model(calibration: CalibrationInput) -> CapabilityCalibrationConfig:
    return CapabilityCalibrationConfig.model_validate(
        {
            "version": calibration.version,
            "slope": parse_float(calibration.slope, "calibration.slope"),
            "intercept": parse_float(calibration.intercept, "calibration.intercept"),
        }
    )


def capability_config(config: ConfigInput) -> CapabilityClassifierConfig:
    values: dict[str, object] = {
        "efficient_tier": config.efficient_tier,
        "capable_tier": config.capable_tier,
        "base_threshold": parse_float(config.base_threshold, "base_threshold"),
    }
    if config.threshold_step is not None:
        values["threshold_step"] = parse_float(config.threshold_step, "threshold_step")
    if config.max_output_tokens is not None:
        values["max_output_tokens"] = config.max_output_tokens
    if config.calibration is not None:
        values["calibration"] = calibration_model(config.calibration)
    if config.response_format is not None:
        values["response_format"] = config.response_format
    return CapabilityClassifierConfig.model_validate(values)


def rendered_format(mode: Mode) -> str:
    body: object = capability_classifier_response_format(mode)
    return json.dumps(body, separators=(",", ":"))


def route_outcome(config: CapabilityClassifierConfig, content: str) -> RoutedRoute | RejectedRoute:
    try:
        verdict = parse_capability_classifier_verdict(content)
    except ValidationError:
        return RejectedRoute(kind="rejected")
    threshold = verdict.routing_threshold(config.base_threshold, config.threshold_step)
    calibration = config.calibration
    calibrated = calibration.calibrate(verdict.p_solve) if calibration is not None else verdict.p_solve
    forecast = CapabilityClassifierForecast(
        verdict=verdict,
        threshold=threshold,
        p_solve=calibrated,
        calibration_version=calibration.version if calibration is not None else None,
    )
    meets = forecast.meets_routing_threshold()
    return RoutedRoute(
        kind="routed",
        tier=config.efficient_tier if meets else config.capable_tier,
        threshold=f"{threshold:.6f}",
        crux=verdict.crux,
        p_solve=f"{forecast.p_solve:.6f}",
        meets=meets,
        primary_rule=verdict.primary_rule,
        capability_boundary=verdict.capability_boundary,
        calibration_version=forecast.calibration_version,
    )


def evaluate(config: ConfigInput, reply: Reply) -> RouteOutcome:
    try:
        checked = capability_config(config)
    except ValidationError:
        return InvalidConfigRoute(kind="invalid_config")
    return route_outcome(checked, reply_content(reply))


def validate(config: ConfigInput) -> ConfigOutcome:
    try:
        checked = capability_config(config)
    except ValidationError:
        return InvalidConfig(kind="invalid")
    version = checked.calibration.version if checked.calibration is not None else None
    return ValidConfig(
        kind="valid",
        efficient_tier=checked.efficient_tier,
        capable_tier=checked.capable_tier,
        base_threshold=f"{checked.base_threshold:.6f}",
        threshold_step=f"{checked.threshold_step:.6f}",
        max_output_tokens=checked.max_output_tokens,
        response_format=checked.response_format,
        calibration_version=version,
    )


class CapabilityForecast(Operation):
    function: Function
    mode: Mode | None = None
    reply: Reply | None = None
    config: ConfigInput | None = None
    calibration: CalibrationInput | None = None

    @override
    def run(self) -> Output:
        match self.function:
            case "response_format":
                return Output(response_format=rendered_format(require(self.mode, "mode")))
            case "system_prompt":
                return Output(
                    system_prompt=capability_classifier_system_prompt(require(self.mode, "mode"))
                )
            case "evaluate":
                return Output(route=evaluate(require(self.config, "config"), require(self.reply, "reply")))
            case "validate_config":
                return Output(config=validate(require(self.config, "config")))
