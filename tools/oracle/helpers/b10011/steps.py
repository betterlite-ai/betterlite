from litellm.proxy._experimental.mcp_server.outbound_credentials.sso_assertion_store import (
    SSOAssertionCache,
    ema_assertion_retention_enabled,
    persist_sso_identity_assertion,
    retain_sso_identity_assertion_for_ema,
)
from pydantic import JsonValue

from b10011.model import ASSERTION_ADAPTER, Step, render_assertion


async def run_step(step: Step, cache: SSOAssertionCache) -> JsonValue:
    if step["step"] == "cache_set":
        cache.set_if_unchanged(
            step["user_id"],
            ASSERTION_ADAPTER.validate_python(step["assertion"]),
            step["seen_epoch"],
        )
        return {"step": "cache_set", "epoch": cache.epoch()}
    if step["step"] == "cache_get":
        found = cache.get(step["user_id"])
        return {
            "step": "cache_get",
            "epoch": cache.epoch(),
            "assertion": None if found is None else render_assertion(found),
        }
    if step["step"] == "invalidate":
        cache.invalidate(step["user_id"])
        return {"step": "invalidate", "epoch": cache.epoch()}
    if step["step"] == "persist":
        await persist_sso_identity_assertion(
            step["user_id"],
            ASSERTION_ADAPTER.validate_python(step["assertion"]),
            cache=cache,
        )
        return {"step": "persist", "epoch": cache.epoch()}
    if step["step"] == "retention":
        return {"step": "retention", "enabled": await ema_assertion_retention_enabled()}
    seed = step["assertion"]
    await retain_sso_identity_assertion_for_ema(
        step["user_id"],
        None if seed is None else ASSERTION_ADAPTER.validate_python(seed),
    )
    return {"step": "retain"}
