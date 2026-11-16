from typing import Final, Literal, TypedDict

from litellm.proxy._experimental.mcp_server.db import McpIdentifierConflict
from litellm.proxy._types import (
    NewMCPServerRequest,
    UpdateMCPServerRequest,
    is_per_server_oauth_discovery_eligible,
)
from litellm.proxy.management_endpoints.mcp_connector_import import (
    ConnectorConversionError,
    ConvertedConnector,
    MCPConnectorImportRequest,
    convert_connector_entries,
    sanitize_connector_name,
)
from litellm.proxy.management_endpoints.mcp_management_endpoints import (
    mcp_identifier_conflict_message,
    stamp_omitted_oauth2_flow,
    validate_and_normalize_mcp_server_payload,
)
from pydantic import BaseModel, JsonValue, TypeAdapter, ValidationError
from starlette.exceptions import HTTPException

OBJECT: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])


class Rejection(BaseModel):
    status_code: int
    detail: dict[str, JsonValue]


class PayloadCall(TypedDict):
    function: Literal["validate_payload", "new_request", "update_request", "stamp_flow"]
    payload: dict[str, JsonValue]


class EligibilityCall(TypedDict):
    function: Literal["eligibility"]
    auth_type: str | None
    oauth2_flow: str | None
    delegate_auth_to_upstream: bool


class ConflictCall(TypedDict):
    function: Literal["conflict_message"]
    field: Literal["server_name", "alias"]
    value: str
    server_id: str


class SanitizeCall(TypedDict):
    function: Literal["sanitize_name"]
    name: str


class ConvertCall(TypedDict):
    function: Literal["convert_connectors"]
    connectors: dict[str, JsonValue]


Input = PayloadCall | EligibilityCall | ConflictCall | SanitizeCall | ConvertCall


def construction_error(error: ValidationError) -> JsonValue:
    message = error.errors()[0]["msg"].removeprefix("Value error, ")
    return {"tag": "error", "message": message}


def validate_payload(call: PayloadCall) -> JsonValue:
    try:
        request = NewMCPServerRequest.model_validate(call["payload"])
    except ValidationError as error:
        return construction_error(error)
    try:
        validate_and_normalize_mcp_server_payload(request)
    except HTTPException as error:
        rejection = Rejection.model_validate(error, from_attributes=True)
        return {"tag": "error", "status": rejection.status_code, "detail": rejection.detail}
    return OBJECT.validate_json(request.model_dump_json())


def new_request(call: PayloadCall) -> JsonValue:
    try:
        request = NewMCPServerRequest.model_validate(call["payload"])
    except ValidationError as error:
        return construction_error(error)
    return OBJECT.validate_json(request.model_dump_json())


def update_request(call: PayloadCall) -> JsonValue:
    try:
        request = UpdateMCPServerRequest.model_validate(call["payload"])
    except ValidationError as error:
        return construction_error(error)
    return OBJECT.validate_json(request.model_dump_json())


def stamp_flow(call: PayloadCall) -> JsonValue:
    try:
        request = NewMCPServerRequest.model_validate(call["payload"])
    except ValidationError as error:
        return construction_error(error)
    stamp_omitted_oauth2_flow(request)
    return OBJECT.validate_json(request.model_dump_json())


def eligibility(call: EligibilityCall) -> JsonValue:
    return {
        "eligible": is_per_server_oauth_discovery_eligible(
            call["auth_type"], call["oauth2_flow"], call["delegate_auth_to_upstream"]
        )
    }


def conflict_message(call: ConflictCall) -> JsonValue:
    conflict = McpIdentifierConflict(
        field=call["field"], value=call["value"], server_id=call["server_id"]
    )
    return {"message": mcp_identifier_conflict_message(conflict)}


def convert_connectors(call: ConvertCall) -> JsonValue:
    request = MCPConnectorImportRequest.model_validate(call["connectors"])
    entries: list[JsonValue] = [outcome(entry) for entry in convert_connector_entries(request)]
    return {"entries": entries}


def outcome(entry: ConvertedConnector | ConnectorConversionError) -> JsonValue:
    match entry:
        case ConvertedConnector():
            request = OBJECT.validate_json(entry.request.model_dump_json())
            return {"name": entry.name, "request": request}
        case ConnectorConversionError():
            return {"name": entry.name, "error": entry.error}


def run(input_value: Input) -> JsonValue:
    result: JsonValue = None
    match input_value["function"]:
        case "validate_payload":
            result = validate_payload(input_value)
        case "new_request":
            result = new_request(input_value)
        case "update_request":
            result = update_request(input_value)
        case "stamp_flow":
            result = stamp_flow(input_value)
        case "eligibility":
            result = eligibility(input_value)
        case "conflict_message":
            result = conflict_message(input_value)
        case "sanitize_name":
            result = {"name": sanitize_connector_name(input_value["name"])}
        case "convert_connectors":
            result = convert_connectors(input_value)
    return result
