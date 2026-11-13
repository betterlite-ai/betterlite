from typing import Final, Literal, NotRequired, TypeAlias, TypedDict  # noqa: I001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/headroom/headroom.py#L81-L321

import httpx
from fastapi.exceptions import HTTPException
from litellm.proxy.guardrails.guardrail_hooks.headroom.headroom import HeadroomGuardrail, _build_compress_failure_detail, _flatten_messages_for_compression, _protected_indices, _raw_retrieve_call_ids, _read_ccr_hashes, _restore_content_shapes, _restore_protected_messages, _retrieval_result_indices  # noqa: E501  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/headroom/headroom.py#L81-L321
from pydantic import JsonValue, TypeAdapter

JSON: Final = TypeAdapter(JsonValue)
API_BASE: Final = "https://headroom.test"


class FlattenInput(TypedDict):
    function: Literal["flatten"]
    rows: list[dict[str, JsonValue]]


class RestoreShapesInput(TypedDict):
    function: Literal["restore_shapes"]
    originals: list[dict[str, JsonValue]]
    returned: list[dict[str, JsonValue]]


class RestoreProtectedInput(TypedDict):
    function: Literal["restore_protected"]
    messages: list[dict[str, JsonValue]]
    compressed: list[dict[str, JsonValue]]
    protected: list[int]


class RetrieveIdsInput(TypedDict):
    function: Literal["retrieve_ids"]
    messages: NotRequired[JsonValue]


class RetrievalResultsInput(TypedDict):
    function: Literal["retrieval_results"]
    messages: list[dict[str, JsonValue]]
    extra_ids: NotRequired[list[str]]


class ProtectedIndicesInput(TypedDict):
    function: Literal["protected_indices"]
    messages: list[dict[str, JsonValue]]
    extra_ids: NotRequired[list[str]]


class FailureDetailInput(TypedDict):
    function: Literal["compress_failure_detail"]
    status: int
    body: str


class HandleFailureInput(TypedDict):
    function: Literal["handle_failure"]
    fallback: Literal["fail_closed", "fail_open"]
    rows: list[dict[str, JsonValue]]
    error: str
    detail: dict[str, JsonValue]


class CcrHashesInput(TypedDict):
    function: Literal["ccr_hashes"]
    body: dict[str, JsonValue]


class TimeoutInput(TypedDict):
    function: Literal["timeout"]
    timeout: float | None


Input: TypeAlias = (
    FlattenInput
    | RestoreShapesInput
    | RestoreProtectedInput
    | RetrieveIdsInput
    | RetrievalResultsInput
    | ProtectedIndicesInput
    | FailureDetailInput
    | HandleFailureInput
    | CcrHashesInput
    | TimeoutInput
)

ShapeInput: TypeAlias = FlattenInput | RestoreShapesInput | RestoreProtectedInput
RetrievalInput: TypeAlias = RetrieveIdsInput | RetrievalResultsInput | ProtectedIndicesInput
FailureInput: TypeAlias = FailureDetailInput | HandleFailureInput | CcrHashesInput


def flatten(input_value: FlattenInput) -> JsonValue:
    flattened = _flatten_messages_for_compression(
        [{key: row[key] for key in row} for row in input_value["rows"]]
    )
    return JSON.validate_python({"rows": flattened})


def restore_shapes(input_value: RestoreShapesInput) -> JsonValue:
    restored = _restore_content_shapes(
        [{key: row[key] for key in row} for row in input_value["originals"]],
        [{key: row[key] for key in row} for row in input_value["returned"]],
    )
    return JSON.validate_python({"rows": restored})


def restore_protected(input_value: RestoreProtectedInput) -> JsonValue:
    restored = _restore_protected_messages(
        [{key: row[key] for key in row} for row in input_value["messages"]],
        [{key: row[key] for key in row} for row in input_value["compressed"]],
        frozenset(input_value["protected"]),
    )
    return JSON.validate_python({"rows": restored})


def retrieve_ids(input_value: RetrieveIdsInput) -> JsonValue:
    ids = _raw_retrieve_call_ids(input_value.get("messages"))
    return JSON.validate_python({"ids": sorted(ids)})


def allowed_ids(input_value: RetrievalResultsInput | ProtectedIndicesInput) -> frozenset[str]:
    ids = input_value.get("extra_ids")
    if ids is None:
        return frozenset()
    return frozenset(ids)


def retrieval_results(input_value: RetrievalResultsInput) -> JsonValue:
    indices = _retrieval_result_indices(
        [{key: row[key] for key in row} for row in input_value["messages"]],
        allowed_ids(input_value),
    )
    return JSON.validate_python({"indices": sorted(indices)})


def protected_indices(input_value: ProtectedIndicesInput) -> JsonValue:
    indices = _protected_indices(
        [{key: row[key] for key in row} for row in input_value["messages"]],
        allowed_ids(input_value),
    )
    return JSON.validate_python({"indices": sorted(indices)})


def compress_failure_detail(input_value: FailureDetailInput) -> JsonValue:
    detail = _build_compress_failure_detail(input_value["status"], input_value["body"])
    return JSON.validate_python({"detail": detail})


def handle_failure(input_value: HandleFailureInput) -> JsonValue:
    guardrail = HeadroomGuardrail(api_base=API_BASE, unreachable_fallback=input_value["fallback"])
    try:
        forwarded = guardrail._handle_compress_failure(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/headroom/headroom.py#L577-L584
            [{key: row[key] for key in row} for row in input_value["rows"]],
            input_value["error"],
            {key: input_value["detail"][key] for key in input_value["detail"]},
        )
    except HTTPException as failure:
        return JSON.validate_python({"status": failure.status_code, "body": failure.detail})
    return JSON.validate_python({"rows": forwarded})


def ccr_hashes(input_value: CcrHashesInput) -> JsonValue:
    hashes = _read_ccr_hashes(input_value["body"])
    return JSON.validate_python({"hashes": sorted(hashes)})


def timeout(input_value: TimeoutInput) -> JsonValue:
    guardrail = HeadroomGuardrail(api_base=API_BASE, timeout=input_value["timeout"])
    resolved = httpx.Timeout(guardrail.timeout).as_dict()
    return JSON.validate_python(
        {"total_seconds": resolved["read"], "connect_seconds": resolved["connect"]}
    )


def shape_result(input_value: ShapeInput) -> JsonValue:
    match input_value["function"]:
        case "flatten":
            return flatten(input_value)
        case "restore_shapes":
            return restore_shapes(input_value)
        case "restore_protected":
            return restore_protected(input_value)


def retrieval_result(input_value: RetrievalInput) -> JsonValue:
    match input_value["function"]:
        case "retrieve_ids":
            return retrieve_ids(input_value)
        case "retrieval_results":
            return retrieval_results(input_value)
        case "protected_indices":
            return protected_indices(input_value)


def failure_result(input_value: FailureInput) -> JsonValue:
    match input_value["function"]:
        case "compress_failure_detail":
            return compress_failure_detail(input_value)
        case "handle_failure":
            return handle_failure(input_value)
        case "ccr_hashes":
            return ccr_hashes(input_value)


def run(input_value: Input) -> JsonValue:
    match input_value["function"]:
        case "flatten" | "restore_shapes" | "restore_protected":
            return shape_result(input_value)
        case "retrieve_ids" | "retrieval_results" | "protected_indices":
            return retrieval_result(input_value)
        case "compress_failure_detail" | "handle_failure" | "ccr_hashes":
            return failure_result(input_value)
        case "timeout":
            return timeout(input_value)
