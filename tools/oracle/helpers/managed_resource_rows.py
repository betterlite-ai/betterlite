from collections.abc import Callable
from typing import TypedDict

from litellm.models.autorouter_session import LiteLLM_AutoRouterSession as RouterRow
from litellm.models.managed_files import LiteLLM_ManagedFileTable as ManagedFile
from litellm.models.managed_files import LiteLLM_ManagedObjectTable as ManagedObject
from litellm.models.managed_files import LiteLLM_ManagedVectorStoresTable as ManagedStores
from litellm.models.managed_files import LiteLLM_ManagedVectorStoreTable as ManagedStore
from litellm.models.mcp_server import LiteLLM_MCPServerTable as McpServerRow
from litellm.models.mcp_server import MCPEnvVar as McpEnvVarRow
from litellm.models.skills import LiteLLM_SkillsTable as SkillRow
from litellm.models.spend_logs import LiteLLM_ErrorLogs as ErrorRow
from litellm.models.spend_logs import LiteLLM_SpendLogs as SpendRow
from litellm.models.tag import LiteLLM_TagTable as TagRow
from pydantic import BaseModel, JsonValue, TypeAdapter

DUMP = TypeAdapter(dict[str, JsonValue])

managed_file_model: type[BaseModel]
managed_object_model: type[BaseModel]
managed_store_model: type[BaseModel]
managed_stores_model: type[BaseModel]
mcp_env_var_model: type[BaseModel]
mcp_server_model: type[BaseModel]
skill_model: type[BaseModel]
router_model: type[BaseModel]
spend_model: type[BaseModel]
error_model: type[BaseModel]
tag_model: type[BaseModel]
(
    managed_file_model,
    managed_object_model,
    managed_store_model,
    managed_stores_model,
    mcp_env_var_model,
    mcp_server_model,
    skill_model,
    router_model,
    spend_model,
    error_model,
    tag_model,
) = (
    ManagedFile,
    ManagedObject,
    ManagedStore,
    ManagedStores,
    McpEnvVarRow,
    McpServerRow,
    SkillRow,
    RouterRow,
    SpendRow,
    ErrorRow,
    TagRow,
)

Reader = Callable[[str, dict[str, JsonValue]], dict[str, JsonValue]]


class RowCall(TypedDict):
    name: str
    stored: str


class Input(TypedDict):
    rows: list[RowCall]


class StoredRow(TypedDict):
    kind: str
    row: dict[str, JsonValue]


STORED = TypeAdapter(StoredRow)


def dumped_file_or_object(kind: str, row: dict[str, JsonValue]) -> dict[str, JsonValue]:
    if kind == "managed_file":
        return DUMP.validate_json(managed_file_model.model_validate(row).model_dump_json())
    return DUMP.validate_json(managed_object_model.model_validate(row).model_dump_json())


def dumped_vector_store(kind: str, row: dict[str, JsonValue]) -> dict[str, JsonValue]:
    if kind == "managed_vector_store":
        return DUMP.validate_json(managed_store_model.model_validate(row).model_dump_json())
    return DUMP.validate_json(managed_stores_model.model_validate(row).model_dump_json())


def dumped_mcp(kind: str, row: dict[str, JsonValue]) -> dict[str, JsonValue]:
    if kind == "mcp_env_var":
        return DUMP.validate_json(mcp_env_var_model.model_validate(row).model_dump_json())
    return DUMP.validate_json(mcp_server_model.model_validate(row).model_dump_json())


def dumped_skill_or_router(kind: str, row: dict[str, JsonValue]) -> dict[str, JsonValue]:
    if kind == "skill":
        return DUMP.validate_json(skill_model.model_validate(row).model_dump_json())
    return DUMP.validate_json(router_model.model_validate(row).model_dump_json())


def dumped_log(kind: str, row: dict[str, JsonValue]) -> dict[str, JsonValue]:
    if kind == "spend_log":
        return DUMP.validate_json(spend_model.model_validate(row).model_dump_json())
    return DUMP.validate_json(error_model.model_validate(row).model_dump_json())


def dumped_tag(kind: str, row: dict[str, JsonValue]) -> dict[str, JsonValue]:
    if kind == "tag":
        return DUMP.validate_json(tag_model.model_validate(row).model_dump_json())
    raise ValueError(f"unknown managed row kind: {kind}")


READERS: dict[str, Reader] = {
    "managed_file": dumped_file_or_object,
    "managed_object": dumped_file_or_object,
    "managed_vector_store": dumped_vector_store,
    "managed_vector_stores": dumped_vector_store,
    "mcp_env_var": dumped_mcp,
    "mcp_server": dumped_mcp,
    "skill": dumped_skill_or_router,
    "autorouter_session": dumped_skill_or_router,
    "spend_log": dumped_log,
    "error_log": dumped_log,
    "tag": dumped_tag,
}


def run(input_value: Input) -> dict[str, list[JsonValue]]:
    results: list[JsonValue] = []
    for call in input_value["rows"]:
        try:
            stored = STORED.validate_json(call["stored"])
            row = READERS[stored["kind"]](stored["kind"], stored["row"])
        except (ValueError, KeyError):
            results.append({"name": call["name"], "ok": False})
            continue
        entry: dict[str, JsonValue] = {"name": call["name"], "ok": True, "row": row}
        if stored["kind"] == "autorouter_session":
            router = RouterRow.model_validate(stored["row"])
            baseline = router.baseline_model
            if baseline is not None:
                entry["baseline_model"] = baseline
        results.append(entry)
    return {"rows": results}
