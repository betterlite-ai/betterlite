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


class ErrorMessage(TypedDict):
    msg: str


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
    first = TypeAdapter(list[ErrorMessage]).validate_json(error.json())[0]
    return {"tag": "error", "message": first["msg"].removeprefix("Value error, ")}


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
            result = {
                "eligible": is_per_server_oauth_discovery_eligible(
                    input_value["auth_type"],
                    input_value["oauth2_flow"],
                    input_value["delegate_auth_to_upstream"],
                )
            }
        case "conflict_message":
            conflict = McpIdentifierConflict(
                field=input_value["field"],
                value=input_value["value"],
                server_id=input_value["server_id"],
            )
            result = {"message": mcp_identifier_conflict_message(conflict)}
        case "sanitize_name":
            result = {"name": sanitize_connector_name(input_value["name"])}
        case "convert_connectors":
            result = convert_connectors(input_value)
    return result
