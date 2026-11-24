import httpx2
from litellm.proxy._experimental.mcp_server.outbound_credentials.adapter import (
    to_server_spec,
    to_subject,
)
from litellm.proxy._types import UserAPIKeyAuth
from litellm.types.mcp_server.mcp_server_manager import MCPServer
from pydantic import JsonValue

from b4667.configs import config_answer
from b4667.wire import JSON_ADAPTER, UPSTREAM_URL, Principal


def to_subject_answer(
    principal: Principal | None, subject_token: str | None
) -> dict[str, JsonValue]:
    auth: UserAPIKeyAuth | None = None
    if principal is not None:
        auth = UserAPIKeyAuth(
            org_id=principal["org_id"],
            team_id=principal["team_id"],
            user_id=principal["user_id"],
        )
    subject = to_subject(auth, subject_token)
    inbound = subject.inbound_token
    return {
        "tenant_id": subject.tenant_id,
        "subject_id": subject.subject_id,
        "inbound_token": None if inbound is None else inbound.get_secret_value(),
    }


def spec_answer(server: MCPServer) -> JsonValue:
    spec = to_server_spec(server)
    if spec is None:
        return None
    return {
        "server_id": spec.server_id,
        "resource": spec.resource,
        "config": config_answer(spec.config),
    }


class HeaderRecorder:
    def __init__(self) -> None:
        self.headers: dict[str, str] = {}

    def __call__(self, request: httpx2.Request) -> httpx2.Response:
        self.headers = dict(request.headers.items())
        return httpx2.Response(200, request=request)


async def applied_headers(auth: httpx2.Auth) -> dict[str, str]:
    recorder = HeaderRecorder()
    transport = httpx2.MockTransport(recorder)
    async with httpx2.AsyncClient(transport=transport, auth=auth) as client:
        default = client.build_request("POST", UPSTREAM_URL)
        await client.post(UPSTREAM_URL)
    return {
        name: value
        for name, value in recorder.headers.items()
        if default.headers.get(name) != value
    }


async def success_record(
    action: str, auth: httpx2.Auth, source: str | None
) -> dict[str, JsonValue]:
    headers = await applied_headers(auth)
    record: dict[str, JsonValue] = {
        "action": action,
        "ok": True,
        "headers": JSON_ADAPTER.validate_python(headers),
    }
    if source is not None:
        record["source"] = source
    return record
