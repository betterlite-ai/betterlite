from types import SimpleNamespace
from typing import Literal, TypeAlias, TypedDict, cast

from litellm.proxy.common_request_processing import (
    serialize_http_exception_detail,  # pyright: ignore[reportUnknownVariableType]
    sse_error_payload,
)
from litellm.proxy.common_utils.sse_keepalive import (
    coerce_keepalive_interval,
    resolve_ttft_keepalive_interval,
)

JsonValue: TypeAlias = None | bool | int | float | str | list["JsonValue"] | dict[str, "JsonValue"]


class CoerceInput(TypedDict):
    function: Literal["coerce"]
    settings: list[float | str | None]


class AgreedGroup(TypedDict):
    deployments: list[dict[str, JsonValue]]
    global_interval: float | str | None


class AgreedInput(TypedDict):
    function: Literal["agreed"]
    groups: list[AgreedGroup]


class DetailPartsInput(TypedDict):
    function: Literal["detail_parts"]
    details: list[JsonValue]


class FailurePayload(TypedDict, total=False):
    status_code: int
    code: str
    error_type: str
    param: str
    detail: JsonValue
    provider_specific_fields: dict[str, JsonValue]


class ErrorPayloadInput(TypedDict):
    function: Literal["error_payload"]
    failures: list[FailurePayload]


Input = CoerceInput | AgreedInput | DetailPartsInput | ErrorPayloadInput


def _exception(failure: FailurePayload) -> SimpleNamespace:
    attributes: dict[str, object] = {}
    if "status_code" in failure:
        attributes["status_code"] = failure["status_code"]
    if "code" in failure:
        attributes["code"] = failure["code"]
    if "error_type" in failure:
        attributes["type"] = failure["error_type"]
    if "param" in failure:
        attributes["param"] = failure["param"]
    if "detail" in failure:
        attributes["detail"] = failure["detail"]
    if "provider_specific_fields" in failure:
        attributes["provider_specific_fields"] = failure["provider_specific_fields"]
    return SimpleNamespace(**attributes)


def _detail_parts(detail: JsonValue) -> dict[str, object]:
    message, fields = cast(
        tuple[str, dict[str, JsonValue] | None],
        serialize_http_exception_detail(detail),
    )
    return {"message": message, "fields": fields}


def _error_payload(failure: FailurePayload) -> dict[str, object]:
    status, body = sse_error_payload(cast(BaseException, _exception(failure)))
    return {"status": status, "body": body}


def run(input_value: Input) -> object:
    match input_value["function"]:
        case "coerce":
            return [
                coerce_keepalive_interval(setting) for setting in input_value["settings"]
            ]
        case "agreed":
            return [
                resolve_ttft_keepalive_interval(
                    group["deployments"], group["global_interval"]
                )
                for group in input_value["groups"]
            ]
        case "detail_parts":
            return [_detail_parts(detail) for detail in input_value["details"]]
        case "error_payload":
            return [_error_payload(failure) for failure in input_value["failures"]]
