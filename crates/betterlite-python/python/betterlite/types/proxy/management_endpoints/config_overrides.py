from betterlite.types.utils import JsonValue, ResponseObject


class HashicorpVaultConfig(ResponseObject):
    vault_addr: str | None = None
    vault_token: str | None = None
    approle_role_id: str | None = None
    approle_secret_id: str | None = None
    approle_mount_path: str | None = None
    client_cert: str | None = None
    client_key: str | None = None
    vault_cert_role: str | None = None
    vault_namespace: str | None = None
    vault_login_namespace: str | None = None
    vault_secret_namespace: str | None = None
    vault_mount_name: str | None = None
    vault_path_prefix: str | None = None


class CyberArkConfig(ResponseObject):
    cyberark_api_base: str | None = None
    cyberark_account: str | None = None
    cyberark_username: str | None = None
    cyberark_api_key: str | None = None
    client_cert: str | None = None
    client_key: str | None = None
    ssl_verify: str | None = None
    refresh_interval: str | None = None


class ConfigOverrideSettingsResponse(ResponseObject):
    config_type: str
    values: dict[str, JsonValue]
    field_schema: dict[str, JsonValue]
