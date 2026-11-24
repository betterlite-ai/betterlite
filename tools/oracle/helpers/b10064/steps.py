from litellm.proxy._experimental.mcp_server.outbound_credentials.sso_assertion_store import (
    AssertionStoreUnavailable,
    DbSSOAssertionStore,
    SSOAssertionCache,
    SSOIdentityAssertion,
    persist_sso_identity_assertion,
)

from b10064.model import (
    ASSERTION_ADAPTER,
    Step,
    StepResult,
    cached_assertion,
    render_assertion,
)
from b10064.table import CountingAssertions


async def run_step(
    step: Step,
    store: DbSSOAssertionStore,
    cache: SSOAssertionCache,
    assertions: CountingAssertions,
) -> StepResult:
    if step["step"] == "fetch":
        pending = step.get("invalidate_during_read")
        assertions.pending_invalidate = step["user_id"] if pending else None
        found: SSOIdentityAssertion | None
        try:
            found = await store.fetch(step["user_id"])
            outcome = "ok"
        except AssertionStoreUnavailable:
            found = None
            outcome = "unavailable"
        assertions.pending_invalidate = None
        return {
            "step": "fetch",
            "outcome": outcome,
            "assertion": None if found is None else render_assertion(found),
            "cached": cached_assertion(cache, step["user_id"]),
            "epoch": cache.epoch(),
            "reads": assertions.reads,
            "writes": assertions.writes,
        }
    if step["step"] == "fetch_uncached":
        uncached: SSOIdentityAssertion | None
        try:
            uncached = await store.fetch_uncached(step["user_id"])
            outcome = "ok"
        except AssertionStoreUnavailable:
            uncached = None
            outcome = "unavailable"
        return {
            "step": "fetch_uncached",
            "outcome": outcome,
            "assertion": None if uncached is None else render_assertion(uncached),
            "cached": cached_assertion(cache, step["user_id"]),
            "epoch": cache.epoch(),
            "reads": assertions.reads,
            "writes": assertions.writes,
        }
    await persist_sso_identity_assertion(
        step["user_id"],
        ASSERTION_ADAPTER.validate_python(step["assertion"]),
        cache=cache,
    )
    return {
        "step": "persist",
        "outcome": "ok",
        "assertion": None,
        "cached": cached_assertion(cache, step["user_id"]),
        "epoch": cache.epoch(),
        "reads": assertions.reads,
        "writes": assertions.writes,
    }
