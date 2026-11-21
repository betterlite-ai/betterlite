from io import BytesIO
from typing import Final, Literal, TypedDict

from fastapi import HTTPException
from litellm.exceptions import (
    BlockedPiiEntityError,
    GuardrailRaisedException,
    ModifyResponseException,
    SensitiveDataRouteException,
)
from litellm.proxy.openai_files_endpoints import batch_guardrails
from litellm.proxy.openai_files_endpoints.batch_guardrails import (
    BatchScanFailure,
    BatchScanResult,
    RecordDropped,
    RecordRedacted,
    UnparseableRecord,
    UnroutableRecord,
    UnscannableRecord,
    build_scan_metadata,
    raise_nothing_to_submit,
    raise_public,
)
from pydantic import JsonValue, TypeAdapter
from starlette.exceptions import HTTPException as RaisedHTTPException

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)


class Item(TypedDict):
    line_number: int
    custom_id: str | None
    url: str | None
    guardrail_name: str | None
    detail_guardrail_name: str | None
    blocked_content: bool
    status: int
    cause: Literal["uncaused", "explicit", "implicit"]
    offset: int
    length: int


class RaisedItem(Item):
    kind: Literal[
        "guardrail_raised",
        "blocked_pii",
        "modify_response",
        "sensitive_data_route",
        "http",
        "other",
    ]


class ChangeItem(Item):
    kind: Literal["redacted", "dropped"]


class FailureItem(Item):
    kind: Literal["unparseable", "unscannable", "unroutable", "nothing_to_submit"]


class CallTypeCall(TypedDict):
    function: Literal["call_type"]
    url: JsonValue
    body: dict[str, JsonValue]


class ScanMetadataCall(TypedDict):
    function: Literal["scan_metadata"]
    metadata: dict[str, JsonValue]


class CustomIdCall(TypedDict):
    function: Literal["custom_id"]
    line: str


class FingerprintCall(TypedDict):
    function: Literal["fingerprint"]
    left: str
    right: str


class ContentBlockCall(TypedDict):
    function: Literal["content_block"]
    raised: RaisedItem


class ReportCall(TypedDict):
    function: Literal["report"]
    scanned_records: int
    changes: list[ChangeItem]


class PublicFailureCall(TypedDict):
    function: Literal["public_failure"]
    failure: FailureItem


Call = (
    CallTypeCall
    | ScanMetadataCall
    | CustomIdCall
    | FingerprintCall
    | ContentBlockCall
    | ReportCall
    | PublicFailureCall
)


class Input(TypedDict):
    calls: list[Call]


class NamedHTTPError(HTTPException):
    def __init__(self, status_code: int, detail: JsonValue, guardrail_name: str | None) -> None:
        super().__init__(status_code=status_code, detail=detail)
        self.guardrail_name = guardrail_name


def run(input_value: Input) -> JsonValue:
    answers: list[JsonValue] = []
    for call in input_value["calls"]:
        if call["function"] == "call_type":
            answers.append(batch_guardrails._scannable_call_type(call["url"], call["body"]))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/openai_files_endpoints/batch_guardrails.py#L310
        elif call["function"] == "scan_metadata":
            narrowed = build_scan_metadata(call["metadata"])
            answers.append(JSON.validate_python({key: narrowed[key] for key in narrowed}))
        elif call["function"] == "custom_id":
            record = next(batch_guardrails._iter_records(BytesIO(call["line"].encode())))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/openai_files_endpoints/batch_guardrails.py#L277
            custom_id = batch_guardrails._custom_id_of(record.payload)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/openai_files_endpoints/batch_guardrails.py#L322
            described = batch_guardrails._describe(custom_id)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/openai_files_endpoints/batch_guardrails.py#L256
            answers.append({"custom_id": custom_id, "described": described})
        elif call["function"] == "fingerprint":
            answers.append(fingerprints_match(call))
        elif call["function"] == "content_block":
            answers.append(content_block(call["raised"]))
        elif call["function"] == "report":
            answers.append(report(call))
        else:
            answers.append(public_failure(call["failure"]))
    return answers


def fingerprints_match(call: FingerprintCall) -> JsonValue:
    lines = BytesIO(f"{call['left']}\n{call['right']}\n".encode())
    left, right = batch_guardrails._iter_records(lines)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/openai_files_endpoints/batch_guardrails.py#L277
    keys = frozenset(left.payload) | frozenset(right.payload)
    left_print = batch_guardrails._fingerprint(left.payload, keys)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/openai_files_endpoints/batch_guardrails.py#L337
    right_print = batch_guardrails._fingerprint(right.payload, keys)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/openai_files_endpoints/batch_guardrails.py#L337
    return {"same": left_print == right_print}


def content_block(item: RaisedItem) -> JsonValue:
    name = item["guardrail_name"]
    detail_name = item["detail_guardrail_name"]
    detail: JsonValue = "blocked"
    if detail_name is not None:
        detail = {"error": "blocked", "guardrail_name": detail_name}
    raised: Exception
    if item["kind"] == "guardrail_raised":
        blocked_content = item["blocked_content"]
        raised = GuardrailRaisedException(guardrail_name=name, blocked_content=blocked_content)
    elif item["kind"] == "blocked_pii":
        raised = BlockedPiiEntityError(entity_type="EMAIL_ADDRESS", guardrail_name=name)
    elif item["kind"] == "modify_response":
        request_data: dict[str, JsonValue] = {}
        raised = ModifyResponseException(
            message="blocked", model="m", request_data=request_data, guardrail_name=name
        )
    elif item["kind"] == "sensitive_data_route":
        raised = SensitiveDataRouteException(
            route_to_model="local", session_id="s", guardrail_name=name
        )
    elif item["kind"] == "http":
        raised = NamedHTTPError(item["status"], detail, name)
    else:
        raised = ValueError("not a guardrail failure")
    if item["cause"] == "explicit":
        raised.__cause__ = ValueError("backend unreachable")
    if item["cause"] == "implicit":
        raised.__context__ = ValueError("unrelated")
    blocked = batch_guardrails._is_content_block(raised)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/openai_files_endpoints/batch_guardrails.py#L223
    guardrail = batch_guardrails._naming_guardrail(raised)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/openai_files_endpoints/batch_guardrails.py#L246
    return {"blocked": blocked, "guardrail": guardrail}


def report(call: ReportCall) -> JsonValue:
    changes: list[RecordRedacted | RecordDropped] = []
    for item in call["changes"]:
        line_number = item["line_number"]
        custom_id = item["custom_id"]
        if item["kind"] == "redacted":
            changes.append(RecordRedacted(line_number, custom_id, item["offset"], item["length"]))
        else:
            changes.append(RecordDropped(line_number, custom_id, item["guardrail_name"]))
    result = BatchScanResult(tuple(changes), call["scanned_records"], BytesIO())
    return {
        "submitted_records": result.submitted_records,
        "report": JSON.validate_json(result.report().model_dump_json()),
        "summary": result.summary(),
    }


def public_failure(item: FailureItem) -> JsonValue:
    line_number = item["line_number"]
    custom_id = item["custom_id"]
    failures: dict[str, BatchScanFailure] = {
        "unparseable": UnparseableRecord(line_number),
        "unscannable": UnscannableRecord(line_number, custom_id, item["url"]),
        "unroutable": UnroutableRecord(line_number, custom_id, item["guardrail_name"]),
    }
    status: int | None = None
    detail: JsonValue = None
    try:
        if item["kind"] == "nothing_to_submit":
            raise_nothing_to_submit()
        raise_public(failures[item["kind"]])
    except RaisedHTTPException as error:
        status = error.status_code
        detail = JSON.validate_python(error.detail)
    return {"status": status, "detail": detail}
