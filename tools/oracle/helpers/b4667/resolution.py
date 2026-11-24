from litellm.proxy._experimental.mcp_server.outbound_credentials.adapter import (
    to_server_spec,
)
from litellm.proxy._experimental.mcp_server.outbound_credentials.resolver import (
    UpstreamCredentialProvider,
    resolve_credentials_with_source,
)
from litellm.proxy._experimental.mcp_server.outbound_credentials.result import (
    Error,
    Ok,
)
from litellm.proxy._experimental.mcp_server.outbound_credentials.types import (
    ServerSpec,
    Subject,
)
from pydantic import JsonValue, SecretStr

from b4667.answers import success_record
from b4667.errors import error_record
from b4667.provider import build_provider
from b4667.wire import JSON_ADAPTER, SERVER_ADAPTER, Action, ResolverCall


async def resolve_actions(
    provider: UpstreamCredentialProvider,
    subject: Subject,
    spec: ServerSpec,
    actions: list[Action],
) -> list[dict[str, JsonValue]]:
    records: list[dict[str, JsonValue]] = []
    for action in actions:
        match action:
            case "resolve":
                match await provider.resolve_credentials(subject, spec):
                    case Error(error):
                        records.append(error_record("resolve", error))
                    case Ok(auth):
                        records.append(await success_record("resolve", auth, None))
            case "resolve_with_source":
                match await resolve_credentials_with_source(provider, subject, spec):
                    case Error(error):
                        records.append(error_record("resolve_with_source", error))
                    case Ok(resolved):
                        record = await success_record(
                            "resolve_with_source", resolved.auth, resolved.source.value
                        )
                        records.append(record)
            case "has_user_token":
                present = await provider.has_user_token(subject, spec)
                records.append(
                    {"action": "has_user_token", "ok": True, "has_user_token": present}
                )
            case "invalidate":
                await provider.invalidate_credentials(subject, spec)
                records.append({"action": "invalidate", "ok": True})
    return records


async def resolver_answer(call: ResolverCall) -> JsonValue:
    spec = to_server_spec(SERVER_ADAPTER.validate_python(call["server"]))
    if spec is None:
        return None
    provider, endpoint, exchanger = build_provider(call.get("collaborators"))
    inbound = call["subject"]["inbound_token"]
    subject = Subject(
        tenant_id=call["subject"]["tenant_id"],
        subject_id=call["subject"]["subject_id"],
        inbound_token=SecretStr(inbound) if inbound is not None else None,
    )
    records = await resolve_actions(provider, subject, spec, call["actions"])
    return {
        "actions": JSON_ADAPTER.validate_python(records),
        "token_endpoint_calls": JSON_ADAPTER.validate_python(endpoint.calls),
        "exchanger_exchanges": JSON_ADAPTER.validate_python(exchanger.exchanges),
        "exchanger_invalidations": JSON_ADAPTER.validate_python(exchanger.invalidations),
    }
