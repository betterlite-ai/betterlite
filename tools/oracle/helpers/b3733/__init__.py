import asyncio
import base64
from collections.abc import Sequence
from typing import Final, Literal, Protocol, TypeAlias, TypedDict, runtime_checkable

from litellm.proxy.guardrails.guardrail_hooks.model_armor import file_scanning
from pydantic import BaseModel, ConfigDict, JsonValue

from .sanitize import SanitizeCall, sanitize


class ScanAttachment(Protocol):
    file_bytes: bytes
    byte_data_type: str


class ScanPlan(Protocol):
    attachments: Sequence[ScanAttachment]
    unscannable_count: int


class PlanFunction(Protocol):
    def __call__(self, messages: Sequence[JsonValue]) -> ScanPlan: ...


@runtime_checkable
class FileScanning(Protocol):
    plan_file_scans: PlanFunction


class FileScanningRecord(BaseModel):
    model_config = ConfigDict(arbitrary_types_allowed=True)
    module: FileScanning


# LiteLLM types plan_file_scans with AllMessageValues, whose message TypedDicts carry Any
FILE_SCANNING_VALUES: Final = {"module": file_scanning}
FILE_SCANNING: Final = FileScanningRecord.model_validate(FILE_SCANNING_VALUES).module


class PlanCall(TypedDict):
    function: Literal["plan_file_scans"]
    messages: list[JsonValue]


def plan(call: PlanCall) -> JsonValue:
    planned = FILE_SCANNING.plan_file_scans(call["messages"])
    attachments: list[JsonValue] = [
        {
            "byte_data_type": attachment.byte_data_type,
            "file_bytes": base64.b64encode(attachment.file_bytes).decode("ascii"),
        }
        for attachment in planned.attachments
    ]
    return {"attachments": attachments, "unscannable_count": planned.unscannable_count}


Input: TypeAlias = PlanCall | SanitizeCall


def run(input_value: Input) -> JsonValue:
    answer: JsonValue = None
    match input_value["function"]:
        case "plan_file_scans":
            answer = plan(input_value)
        case "sanitize_request":
            answer = asyncio.run(sanitize(input_value))
    return answer
