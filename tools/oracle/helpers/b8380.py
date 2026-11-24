from typing import TypedDict

from litellm.proxy import proxy_server
from litellm.proxy.config_resolvers.settings_rules import JsonValue, Section
from litellm.proxy.config_resolvers.settings_store import SettingsStore
from pydantic import BaseModel
from starlette.exceptions import HTTPException


class Input(TypedDict):
    operation: str
    section: Section
    yaml_values: dict[str, JsonValue]
    database_row: dict[str, JsonValue] | None
    changed_keys: dict[str, JsonValue]
    config_file_path: str


class ConfigOwnedDetail(TypedDict):
    error: str
    keys: list[str]
    section: str
    stored_database_values_ignored: list[str]
    resolution: str


class ConfigOwnedError(BaseModel):
    status_code: int
    detail: ConfigOwnedDetail


class Answer(TypedDict):
    status_code: int | None
    detail: ConfigOwnedDetail | None


def store_for(config: proxy_server.ProxyConfig, section: Section) -> SettingsStore | None:
    if section == "general_settings":
        return config.settings
    if section == "router_settings":
        return config.router_settings
    if section == "litellm_settings":
        return config.litellm_settings
    if section == "environment_variables":
        return config.environment_variables
    return None


# reject_config_owned_writes: None when the store lets the write through, otherwise the 400 detail
def owned_writes(input_value: Input) -> Answer:
    proxy_server.user_config_file_path = input_value["config_file_path"]
    config = proxy_server.ProxyConfig()
    store = store_for(config, input_value["section"])
    if store is not None:
        store.load_yaml(input_value["yaml_values"])
        database_row: dict[str, JsonValue] | None = input_value["database_row"]
        if database_row is not None:
            store.apply_db_row(input_value["section"], database_row)
    try:
        config.reject_config_owned_writes(
            section_name=input_value["section"],
            changed_keys=input_value["changed_keys"],
        )
    except HTTPException as error:
        rejected = ConfigOwnedError.model_validate(error, from_attributes=True)
        return {"status_code": rejected.status_code, "detail": rejected.detail}
    return {"status_code": None, "detail": None}


def run(input_value: Input) -> Answer:
    if input_value["operation"] == "owned_writes":
        return owned_writes(input_value)
    raise ValueError(f"unknown b8380 operation: {input_value['operation']}")
