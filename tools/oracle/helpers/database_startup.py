import os
from typing import Literal, NotRequired, TypedDict

from litellm.proxy.db.prisma_client import should_update_prisma_schema
from litellm_proxy_extras.prisma_toolchain import (
    migration_lock_timeout,
    prisma_command_timeout,
    prisma_migrate_deploy_timeout,
)


class Input(TypedDict):
    function: Literal["settings"]
    environment: dict[str, str]
    disable_prisma_schema_update: NotRequired[bool | str | None]


class Settings(TypedDict):
    updates_enabled: bool
    command_timeout: float
    lock_timeout: float
    deploy_timeout: float


def run(input_value: Input) -> Settings:
    os.environ.clear()
    os.environ.update(input_value["environment"])
    return Settings(
        updates_enabled=should_update_prisma_schema(
            input_value.get("disable_prisma_schema_update")
        ),
        command_timeout=prisma_command_timeout(),
        lock_timeout=migration_lock_timeout(),
        deploy_timeout=prisma_migrate_deploy_timeout(),
    )
