from typing import Final, Literal, TypeAlias, TypedDict

from litellm.integrations.shadow_eval_logger import (
    GuardrailRequestSnapshot,
    _request_mutating_guardrail_ran,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/shadow_eval_logger.py#L321-L328
    request_guardrail_fingerprint,
)
from pydantic import JsonValue, TypeAdapter

JSON: Final = TypeAdapter(JsonValue)


class MutatingRanInput(TypedDict):
    function: Literal["mutating_ran"]
    metadata: dict[str, JsonValue]


class FingerprintsMatchInput(TypedDict):
    function: Literal["fingerprints_match"]
    first: dict[str, JsonValue]
    second: dict[str, JsonValue]


class CaptureInput(TypedDict):
    function: Literal["capture"]
    body: dict[str, JsonValue]
    metadata: dict[str, JsonValue]


Input: TypeAlias = MutatingRanInput | FingerprintsMatchInput | CaptureInput


def run(input_value: Input) -> JsonValue:
    match input_value["function"]:
        case "mutating_ran":
            return {"result": _request_mutating_guardrail_ran(input_value["metadata"])}
        case "fingerprints_match":
            first = request_guardrail_fingerprint(input_value["first"])
            second = request_guardrail_fingerprint(input_value["second"])
            return {"result": first == second}
        case "capture":
            snapshot = GuardrailRequestSnapshot.capture(
                input_value["body"], input_value["metadata"]
            )
            if snapshot is None:
                return {"result": None}
            current = request_guardrail_fingerprint(input_value["metadata"])
            return {
                "result": {
                    "body": JSON.validate_python(dict(snapshot.body)),
                    "fingerprint_matches": snapshot.fingerprint == current,
                }
            }
