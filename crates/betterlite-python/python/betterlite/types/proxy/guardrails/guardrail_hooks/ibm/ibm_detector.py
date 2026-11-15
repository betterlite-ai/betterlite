from pydantic import Field, JsonValue
from typing_extensions import TypedDict, override

from betterlite.types.proxy.guardrails.guardrail_hooks.base import GuardrailConfigModel
from betterlite.types.proxy.guardrails.guardrail_hooks.ibm.base import (
    IBMGuardrailsBaseConfigModel,
)
from betterlite.types.utils.base import ResponseObject


# ast-grep-ignore: no-long-names
class IBMDetectorRequestBodyDetectorServer(TypedDict):
    contents: list[str]
    detector_params: dict[str, JsonValue]


# ast-grep-ignore: no-long-names
class IBMDetectorRequestBodyOrchestrator(TypedDict):
    content: str
    detectors: dict[str, dict[str, JsonValue]]


class IBMDetectorDetection(TypedDict, total=False):
    start: int
    end: int
    text: str
    detection: str
    detection_type: str
    score: float
    evidences: list[JsonValue]
    metadata: dict[str, JsonValue]
    detector_id: str | None


# ast-grep-ignore: no-long-names
class IBMDetectorResponseDetectorServer(TypedDict):
    detections: list[list[IBMDetectorDetection]]


# ast-grep-ignore: no-long-names
class IBMDetectorResponseOrchestrator(TypedDict):
    detections: list[IBMDetectorDetection]


class IBMDetectorOptionalParams(ResponseObject):
    detector_params: dict[str, JsonValue] | None = Field(default_factory=dict)
    extra_headers: dict[str, JsonValue] | None = Field(default_factory=dict)
    score_threshold: float | None = None
    block_on_detection: bool | None = True


# ast-grep-ignore: no-long-names
class IBMDetectorGuardrailConfigModel(
    IBMGuardrailsBaseConfigModel,
    GuardrailConfigModel[IBMDetectorOptionalParams],
):
    optional_params: IBMDetectorOptionalParams | None = Field(
        default_factory=IBMDetectorOptionalParams,
    )

    @staticmethod
    @override
    def ui_friendly_name() -> str:
        return "IBM Guardrails Detector"
