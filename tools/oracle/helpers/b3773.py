from typing import Final, Literal, NotRequired, TypedDict  # noqa: I001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/headroom/headroom.py#L81-L321

import httpx
from litellm.proxy.guardrails.guardrail_hooks.headroom.headroom import HeadroomGuardrail, _build_compress_failure_detail, _flatten_messages_for_compression, _protected_indices, _raw_retrieve_call_ids, _read_ccr_hashes, _restore_content_shapes, _restore_protected_messages, _retrieval_result_indices  # noqa: E501  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/headroom/headroom.py#L81-L321
from pydantic import JsonValue, TypeAdapter
from starlette.exceptions import HTTPException as StarletteHTTPException

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


Input = (
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


def flatten(input_value: FlattenInput) -> JsonValue:
    flattened = _flatten_messages_for_compression(
        [{key: row[key] for key in row} for row in input_value["rows"]]
    )
    return {"rows": JSON.validate_python(flattened)}


def restore_shapes(input_value: RestoreShapesInput) -> JsonValue:
    restored = _restore_content_shapes(
        [{key: row[key] for key in row} for row in input_value["originals"]],
        [{key: row[key] for key in row} for row in input_value["returned"]],
    )
    return {"rows": JSON.validate_python(restored)}


def restore_protected(input_value: RestoreProtectedInput) -> JsonValue:
    restored = _restore_protected_messages(
        [{key: row[key] for key in row} for row in input_value["messages"]],
        [{key: row[key] for key in row} for row in input_value["compressed"]],
        frozenset(input_value["protected"]),
    )
    return {"rows": JSON.validate_python(restored)}


def retrieve_ids(input_value: RetrieveIdsInput) -> JsonValue:
    ids = sorted(_raw_retrieve_call_ids(input_value.get("messages")))
    return {"ids": JSON.validate_python(ids)}


def allowed_ids(input_value: RetrievalResultsInput | ProtectedIndicesInput) -> frozenset[str]:
    ids = input_value.get("extra_ids")
    if ids is None:
        return frozenset()
    return frozenset(ids)


def retrieval_results(input_value: RetrievalResultsInput) -> JsonValue:
    found = _retrieval_result_indices(
        [{key: row[key] for key in row} for row in input_value["messages"]],
        allowed_ids(input_value),
    )
    indices = sorted(found)
    return {"indices": JSON.validate_python(indices)}


def protected_indices(input_value: ProtectedIndicesInput) -> JsonValue:
    found = _protected_indices(
        [{key: row[key] for key in row} for row in input_value["messages"]],
        allowed_ids(input_value),
    )
    indices = sorted(found)
    return {"indices": JSON.validate_python(indices)}


def compress_failure_detail(input_value: FailureDetailInput) -> JsonValue:
    detail = _build_compress_failure_detail(input_value["status"], input_value["body"])
    return {"detail": JSON.validate_python(detail)}


def handle_failure(input_value: HandleFailureInput) -> JsonValue:
    guardrail = HeadroomGuardrail(api_base=API_BASE, unreachable_fallback=input_value["fallback"])
    try:
        forwarded = guardrail._handle_compress_failure(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/headroom/headroom.py#L577-L584
            [{key: row[key] for key in row} for row in input_value["rows"]],
            input_value["error"],
            {key: input_value["detail"][key] for key in input_value["detail"]},
        )
    except StarletteHTTPException as failure:
        return {"status": failure.status_code, "body": failure.detail}
    return {"rows": JSON.validate_python(forwarded)}


def ccr_hashes(input_value: CcrHashesInput) -> JsonValue:
    hashes = sorted(_read_ccr_hashes(input_value["body"]))
    return {"hashes": JSON.validate_python(hashes)}


def timeout(input_value: TimeoutInput) -> JsonValue:
    guardrail = HeadroomGuardrail(api_base=API_BASE, timeout=input_value["timeout"])
    budget = httpx.Timeout(guardrail.timeout).as_dict()
    return {"total_seconds": budget["read"], "connect_seconds": budget["connect"]}


def run(input_value: Input) -> JsonValue:
    result: JsonValue
    if input_value["function"] == "flatten":
        result = flatten(input_value)
    elif input_value["function"] == "restore_shapes":
        result = restore_shapes(input_value)
    elif input_value["function"] == "restore_protected":
        result = restore_protected(input_value)
    elif input_value["function"] == "retrieve_ids":
        result = retrieve_ids(input_value)
    elif input_value["function"] == "retrieval_results":
        result = retrieval_results(input_value)
    elif input_value["function"] == "protected_indices":
        result = protected_indices(input_value)
    elif input_value["function"] == "compress_failure_detail":
        result = compress_failure_detail(input_value)
    elif input_value["function"] == "handle_failure":
        result = handle_failure(input_value)
    elif input_value["function"] == "ccr_hashes":
        result = ccr_hashes(input_value)
    else:
        result = timeout(input_value)
    return result
