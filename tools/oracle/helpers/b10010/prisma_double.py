import sys
from collections.abc import Mapping, Sequence
from types import ModuleType
from typing import Final

from litellm.proxy.db.prisma_client import PrismaWrapper
from litellm.proxy.utils import PrismaClient
from typing_extensions import override

MODELS_MODULE: Final = "prisma.models"
ASSERTION_MODEL: Final = "LiteLLM_SSOIdentityAssertion"


class AssertionRow:
    def __init__(self, user_id: str, assertion_b64: str) -> None:
        self.user_id = user_id
        self.assertion_b64 = assertion_b64


class ServerRow:
    def __init__(self, server_id: str, auth_type: str | None) -> None:
        self.server_id = server_id
        self.auth_type = auth_type


class ModelsModule(ModuleType):
    @override
    def __getattr__(self, name: str) -> type[AssertionRow]:
        if name == ASSERTION_MODEL:
            return AssertionRow
        raise AttributeError(name)


def install_models() -> None:
    models = ModelsModule(MODELS_MODULE)
    sys.modules.setdefault(MODELS_MODULE, models)


# the store imports prisma.models inside the call, and no client is generated here
install_models()


class AssertionTable:
    def __init__(self, rows: list[AssertionRow]) -> None:
        self.rows = rows
        self.updates: list[tuple[str, str]] = []

    async def find_unique(self, *, where: Mapping[str, str]) -> AssertionRow | None:
        for row in self.rows:
            if row.user_id == where["user_id"]:
                return row
        return None

    async def find_many(self) -> Sequence[AssertionRow]:
        return list(self.rows)

    async def upsert(
        self,
        *,
        where: Mapping[str, str],
        data: Mapping[str, Mapping[str, str]],
    ) -> None:
        found = await self.find_unique(where=where)
        if found is None:
            self.rows.append(AssertionRow(where["user_id"], data["create"]["assertion_b64"]))
            return
        found.assertion_b64 = data["update"]["assertion_b64"]

    async def update(self, *, where: Mapping[str, str], data: Mapping[str, str]) -> None:
        self.updates.append((where["user_id"], data["assertion_b64"]))
        found = await self.find_unique(where=where)
        if found is None:
            self.rows.append(AssertionRow(where["user_id"], data["assertion_b64"]))
            return
        found.assertion_b64 = data["assertion_b64"]


class ServerTable:
    def __init__(self, rows: list[ServerRow]) -> None:
        self.rows = rows

    async def find_first(self, *, where: Mapping[str, str]) -> ServerRow | None:
        for row in self.rows:
            if row.auth_type == where["auth_type"]:
                return row
        return None


class FakeDb(PrismaWrapper):
    def __init__(self, assertions: AssertionTable, servers: ServerTable) -> None:
        self.litellm_ssoidentityassertion = assertions
        self.litellm_mcpservertable = servers


class FakePrismaClient(PrismaClient):
    def __init__(self, assertions: AssertionTable, servers: ServerTable) -> None:
        self.db = FakeDb(assertions, servers)
