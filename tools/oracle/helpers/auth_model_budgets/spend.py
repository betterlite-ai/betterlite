import asyncio
from typing import Final

import litellm
from litellm.proxy.auth import user_api_key_auth
from pydantic import JsonValue

from .doubles import AlertRecorder, Call, FakeCache, FakePrisma, FakeUserTable


class SpendRunner:
    def __init__(
        self,
        *,
        admin: str,
        cache: FakeCache,
        prisma: FakePrisma | None,
        recorder: AlertRecorder,
    ) -> None:
        self.admin: Final = admin
        self.cache: Final = cache
        self.prisma: Final = prisma
        self.recorder: Final = recorder

    async def spend(self, token: str) -> float | None:
        return await user_api_key_auth.get_global_proxy_spend(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L760-L790
            litellm_proxy_admin_name=self.admin,
            user_api_key_cache=self.cache,  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L762
            prisma_client=self.prisma,  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L763
            token=token,
            proxy_logging_obj=self.recorder,
        )

    async def follower_spend(self, usertable: FakeUserTable, token: str) -> float | None:
        await usertable.started.wait()
        usertable.release.set()
        return await self.spend(token)


async def global_spend_call(call: Call) -> JsonValue:
    max_budget = call.get("max_budget")
    tokens = call.get("tokens")
    admin = call.get("admin_name")
    if max_budget is None or not tokens or admin is None:
        raise ValueError("a global spend call names max_budget, tokens and the admin name")
    litellm.max_budget = max_budget
    usertable = FakeUserTable(spend=call.get("spend"), gated=len(tokens) > 1)
    prisma = None if call.get("database") is False else FakePrisma(usertable)
    recorder = AlertRecorder()
    runner = SpendRunner(admin=admin, cache=FakeCache(), prisma=prisma, recorder=recorder)
    first: float | None
    second: float | None = None
    if len(tokens) > 1:
        first, second = await asyncio.gather(
            runner.spend(tokens[0]),
            runner.follower_spend(usertable, tokens[1]),
        )
    else:
        first = await runner.spend(tokens[0])
    # a scheduled alert task only runs once the request path yields
    await asyncio.sleep(0)
    return {
        "spend": first,
        "second": second,
        "alerts": list(recorder.alerts),
        "reads": len(usertable.reads),
    }
