from collections.abc import Mapping
from typing import Final

from litellm.proxy._experimental.mcp_server.outbound_credentials.sso_assertion_store import (
    SSOAssertionCache,
)
from typing_extensions import override

from b10010.prisma_double import AssertionRow, AssertionTable

FAKE_FAILURE: Final = "the in-memory assertion rows failed on Find"


class CountingAssertions(AssertionTable):
    def __init__(
        self,
        rows: list[AssertionRow],
        failure_call: int | None,
        cache: SSOAssertionCache,
    ) -> None:
        super().__init__(rows)
        self.reads = 0
        self.writes = 0
        self.failure_call = failure_call
        self.cache = cache
        self.pending_invalidate: str | None = None

    @override
    async def find_unique(self, *, where: Mapping[str, str]) -> AssertionRow | None:
        user_id = where["user_id"]
        pending = self.pending_invalidate
        self.pending_invalidate = None
        if pending == user_id:
            self.cache.invalidate(user_id)
        if self.failure_call == self.reads:
            raise RuntimeError(FAKE_FAILURE)
        self.reads += 1
        return self.lookup(user_id)

    @override
    async def upsert(
        self,
        *,
        where: Mapping[str, str],
        data: Mapping[str, Mapping[str, str]],
    ) -> None:
        found = self.lookup(where["user_id"])
        if found is None:
            self.rows.append(AssertionRow(where["user_id"], data["create"]["assertion_b64"]))
        else:
            found.assertion_b64 = data["update"]["assertion_b64"]
        self.writes += 1

    def lookup(self, user_id: str) -> AssertionRow | None:
        for row in self.rows:
            if row.user_id == user_id:
                return row
        return None
