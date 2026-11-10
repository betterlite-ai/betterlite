import asyncio
import json
from types import MappingProxyType
from typing import Literal, TypeAlias, cast

from litellm.proxy._experimental.mcp_server.client_allowlist import (
    MCPClientAllowlist,
    check_mcp_client_allowed,
    load_mcp_client_allowlist,
)
from litellm.proxy._experimental.mcp_server.elicitation_handler import (
    handle_elicitation_request,
)
from mcp.types import (
    ClientCapabilities,
    ElicitRequestParams,
    ElicitResult,
    ErrorData,
)
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter
from starlette.datastructures import Headers

Json: TypeAlias = JsonValue


class Strict(BaseModel):
    model_config = ConfigDict(extra="forbid")


class LoadInput(Strict):
    operation: Literal["allowlist_load"]
    settings: dict[str, Json] = Field(default_factory=dict)


class AllowlistInput(Strict):
    aliases_by_value: dict[str, str] = Field(default_factory=dict)
    jwt_field: str | None = None
    header: str | None = None


class CheckScenario(Strict):
    allowlist: AllowlistInput | None = None
    jwt_claims: dict[str, Json] | None = None
    headers: dict[str, str] = Field(default_factory=dict)


class CheckInput(Strict):
    operation: Literal["allowlist_check"]
    scenarios: list[CheckScenario] = Field(default_factory=list)


class SessionResult(Strict):
    result: dict[str, Json]


class SessionError(Strict):
    error: str


class ElicitScenario(Strict):
    params: dict[str, Json]
    capabilities: dict[str, Json] | None = None
    session: SessionResult | SessionError | None = None


class ElicitationInput(Strict):
    operation: Literal["elicitation"]
    scenarios: list[ElicitScenario] = Field(default_factory=list)


class RecordingSession:
    def __init__(self, behaviour: SessionResult | SessionError) -> None:
        self.behaviour = behaviour
        self.calls: list[dict[str, object]] = []

    def _outcome(self) -> ElicitResult:
        match self.behaviour:
            case SessionResult():
                return ElicitResult.model_validate(self.behaviour.result)
            case SessionError():
                raise RuntimeError(self.behaviour.error)

    async def elicit_url(self, message: str, url: str, elicitation_id: str | None) -> ElicitResult:
        self.calls.append(
            {
                "method": "elicit_url",
                "message": message,
                "url": url,
                "elicitation_id": elicitation_id,
            }
        )
        return self._outcome()

    async def elicit_form(self, message: str, requested_schema: dict[str, object]) -> ElicitResult:
        self.calls.append(
            {
                "method": "elicit_form",
                "message": message,
                "requested_schema": cast(Json, requested_schema),
            }
        )
        return self._outcome()

    async def elicit(self, message: str, requested_schema: dict[str, object]) -> ElicitResult:
        self.calls.append(
            {
                "method": "elicit",
                "message": message,
                "requested_schema": cast(Json, requested_schema),
            }
        )
        return self._outcome()


_ELICIT_PARAMS: TypeAdapter[ElicitRequestParams] = TypeAdapter(ElicitRequestParams)


def _json(value: object) -> Json:
    return cast(Json, json.loads(json.dumps(value)))


def _allowlist(raw: AllowlistInput | None) -> MCPClientAllowlist | None:
    if raw is None:
        return None
    return MCPClientAllowlist(
        aliases_by_value=MappingProxyType(dict(raw.aliases_by_value)),
        jwt_field=raw.jwt_field,
        header=raw.header,
    )


def _load(parsed: LoadInput) -> Json:
    allowlist = load_mcp_client_allowlist(parsed.settings)
    if allowlist is None:
        return {"enforced": False}
    return _json(
        {
            "enforced": True,
            "aliases_by_value": dict(allowlist.aliases_by_value),
            "jwt_field": allowlist.jwt_field,
            "header": allowlist.header,
        }
    )


def _check(scenario: CheckScenario) -> Json:
    rejection = check_mcp_client_allowed(
        _allowlist(scenario.allowlist),
        scenario.jwt_claims,
        Headers(scenario.headers),
    )
    if rejection is None:
        return {"allowed": True}
    return _json(
        {
            "allowed": False,
            "details": rejection.details,
            "body": dict(rejection.response_body),
        }
    )


def _elicit(scenario: ElicitScenario) -> Json:
    capabilities = (
        None
        if scenario.capabilities is None
        else ClientCapabilities.model_validate(scenario.capabilities)
    )
    session = None if scenario.session is None else RecordingSession(scenario.session)
    outcome = asyncio.run(
        handle_elicitation_request(
            None,
            _ELICIT_PARAMS.validate_python(scenario.params),
            session,
            capabilities,
        )
    )
    if isinstance(outcome, ErrorData):
        return _json(
            {"kind": "error", "code": outcome.code, "message": outcome.message}
        )
    output = cast(
        dict[str, Json],
        outcome.model_dump(mode="json", by_alias=True, exclude_none=True),
    )
    output["calls"] = _json([] if session is None else session.calls)
    return _json(output)


def run(input_value: dict[str, Json]) -> Json:
    operation = input_value.get("operation")
    match operation:
        case "allowlist_load":
            return _load(LoadInput.model_validate(input_value))
        case "allowlist_check":
            parsed = CheckInput.model_validate(input_value)
            return {"results": [_check(scenario) for scenario in parsed.scenarios]}
        case "elicitation":
            parsed = ElicitationInput.model_validate(input_value)
            return {"results": [_elicit(scenario) for scenario in parsed.scenarios]}
        case _:
            raise ValueError(f"unknown b8191 operation: {operation}")
