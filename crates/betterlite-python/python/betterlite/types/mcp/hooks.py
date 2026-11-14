from pydantic import BaseModel, Field, JsonValue

from betterlite.types.utils.text_completion import HiddenParams


class MCPPreCallRequestObject(BaseModel):
    tool_name: str
    arguments: dict[str, JsonValue]
    server_name: str | None = None
    user_api_key_auth: dict[str, JsonValue] | None = None
    hidden_params: HiddenParams = HiddenParams()


class MCPPreCallResponseObject(BaseModel):
    should_proceed: bool = True
    modified_arguments: dict[str, JsonValue] | None = None
    error_message: str | None = None
    hidden_params: HiddenParams = HiddenParams()


class MCPDuringCallRequestObject(BaseModel):
    tool_name: str
    arguments: dict[str, JsonValue]
    server_name: str | None = None
    start_time: float | None = None
    hidden_params: HiddenParams = HiddenParams()


class MCPDuringCallResponseObject(BaseModel):
    should_continue: bool = True
    error_message: str | None = None
    hidden_params: HiddenParams = HiddenParams()


class MCPPostCallResponseObject(BaseModel):
    mcp_tool_call_response: list[BaseModel | JsonValue]
    hidden_params: HiddenParams


class MCPGatewaySession(BaseModel):
    session_id_prefix: str
    client_name: str | None = None
    client_version: str | None = None
    user_id: str | None = None
    user_email: str | None = None
    key_alias: str | None = None
    team_id: str | None = None
    team_alias: str | None = None
    client_ip: str | None = None
    idle_seconds: float
    in_flight_requests: int


class MCPGatewaySessionGroupCount(BaseModel):
    label: str | None = None
    count: int


class MCPGatewaySessionsResponse(BaseModel):
    worker_pid: int
    total_sessions: int
    by_client: list[MCPGatewaySessionGroupCount] = Field(default_factory=list)
    by_user: list[MCPGatewaySessionGroupCount] = Field(default_factory=list)
    sessions: list[MCPGatewaySession] = Field(default_factory=list)


class MCPGatewaySessionsTerminateResponse(BaseModel):  # ast-grep-ignore: no-long-names
    worker_pid: int
    terminated_sessions: int
    sessions: list[MCPGatewaySession] = Field(default_factory=list)
