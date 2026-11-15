import asyncio
from logging import Handler, LogRecord, Logger, getLogger
from typing import Final, TypedDict

from litellm.proxy.auth.master_key_boot_check import (
    GENERATE_MASTER_KEY_COMMAND,
    MASTER_KEY_ENV_VAR,
    MIGRATE_FROM_MASTER_KEY_ENV_VAR as MIGRATION_VARIABLE,
    PRINT_NEW_MASTER_KEY_COMMAND,
    ROTATION_DOCS_URL,
    WEAK_OR_UNSET_MASTER_KEY_OVERRIDE_ENV_VAR as OVERRIDE_ENV_VAR,
    WEAK_OR_UNSET_MASTER_KEY_OVERRIDE_SETTING as OVERRIDE_SETTING,
    ConfigFileSource,
    EnvironmentSource,
    MasterKeyBootVerdict,
    MasterKeySource,
    SafeMasterKey,
    StoredSecretsMigration,
    UnsafeMasterKeyAllowed,
    UnsafeMasterKeyError,
    UnsafeMasterKeyReason,
    UnsafeMasterKeyRefused,
    enforce_master_key_boot_verdict,
    master_key_boot_verdict,
    render_refusal,
    with_stored_secrets_counted,
)
from pydantic import JsonValue
from typing_extensions import override

VERBOSE_TAIL: Final = "Never run this outside local development."


class DatabaseInput(TypedDict):
    salt_key_is_set: bool
    is_configured: bool


class Input(TypedDict):
    master_key: str | None
    environment_master_key: str | None
    general_settings: dict[str, JsonValue]
    config_file_path: str | None
    override_env_is_on: bool
    database: DatabaseInput
    stored_secret_count: int | None


class Counting:
    def __init__(self, count: int | None) -> None:
        self.count = count
        self.asked_about: list[JsonValue] = []

    async def __call__(self, signing_key: str) -> int | None:
        self.asked_about.append(signing_key)
        return self.count


class WarningHandler(Handler):
    def __init__(self) -> None:
        super().__init__()
        self.warnings: list[JsonValue] = []

    @override
    def emit(self, record: LogRecord) -> None:
        message = record.getMessage()
        prefix = f"{OVERRIDE_SETTING} is on, so "
        if not message.startswith(prefix):
            return
        entry: dict[str, JsonValue] = {
            "setting": OVERRIDE_SETTING,
            "message_shaped": message.endswith(VERBOSE_TAIL),
        }
        self.warnings.append(entry)


def run(input_value: Input) -> dict[str, JsonValue]:
    verdict = master_key_boot_verdict(
        master_key=input_value["master_key"],
        environment_master_key=input_value["environment_master_key"],
        general_settings=input_value["general_settings"],
        config_file_path=input_value["config_file_path"],
        override_env_is_on=input_value["override_env_is_on"],
        salt_key_is_set=input_value["database"]["salt_key_is_set"],
        database_is_configured=input_value["database"]["is_configured"],
    )
    counting = Counting(input_value["stored_secret_count"])
    counted = asyncio.run(with_stored_secrets_counted(verdict, counting))
    announcements: list[JsonValue] = []
    recorder = WarningHandler()
    logger: Logger = getLogger("LiteLLM Proxy")
    logger.addHandler(recorder)
    error_kind = "none"
    try:
        enforce_master_key_boot_verdict(counted, announcements.append)
    except UnsafeMasterKeyError:
        error_kind = "refused"
    finally:
        logger.removeHandler(recorder)
    return {
        "verdict": verdict_report(counted),
        "counted": {"asked_about": counting.asked_about},
        "warnings": recorder.warnings,
        "announcements": announcement_report(counted, announcements),
        "error_kind": error_kind,
    }


def verdict_report(verdict: MasterKeyBootVerdict) -> dict[str, JsonValue]:
    match verdict:
        case SafeMasterKey():
            return base_report("safe", None)
        case UnsafeMasterKeyAllowed(reason=reason):
            return base_report("allowed", reason)
        case UnsafeMasterKeyRefused() as refused:
            return {
                "kind": "refused",
                "reason": reason_name(refused.reason),
                "source": source_kind(refused.source),
                "config_file_path": config_path(refused.source),
                "environment_variable_is_set": refused.environment_variable_is_set,
                "migration": migration_report(refused.migration),
                "rendered": rendered_report(render_refusal(refused)),
            }


def base_report(kind: str, reason: UnsafeMasterKeyReason | None) -> dict[str, JsonValue]:
    return {
        "kind": kind,
        "reason": reason_name(reason),
        "source": None,
        "config_file_path": None,
        "environment_variable_is_set": None,
        "migration": None,
        "rendered": None,
    }


def reason_name(reason: UnsafeMasterKeyReason | None) -> str | None:
    if reason is None:
        return None
    return reason.name.lower()


def source_kind(source: MasterKeySource) -> str:
    match source:
        case ConfigFileSource():
            return "config_file"
        case EnvironmentSource():
            return "environment"


def config_path(source: MasterKeySource) -> str | None:
    match source:
        case ConfigFileSource(config_file_path=path):
            return path
        case EnvironmentSource():
            return None


def migration_report(migration: StoredSecretsMigration | None) -> dict[str, JsonValue] | None:
    if migration is None:
        return None
    return {
        "from_master_key": migration.from_master_key,
        "encrypted_value_count": migration.encrypted_value_count,
    }


def rendered_report(text: str) -> dict[str, JsonValue]:
    return {
        "steps": numbered_steps(text),
        "save_command": GENERATE_MASTER_KEY_COMMAND in text,
        "print_command": PRINT_NEW_MASTER_KEY_COMMAND in text,
        "migration_line": migration_line(text),
        "rotation_docs": ROTATION_DOCS_URL in text,
        "config_placeholder": f"os.environ/{MASTER_KEY_ENV_VAR}" in text,
        "override_env_hint": f"{OVERRIDE_ENV_VAR}=true" in text,
        "override_setting_hint": f"general_settings.{OVERRIDE_SETTING}: true" in text,
        "override_hint_last": last_paragraph(text).startswith("Local development only: set "),
    }


def numbered_steps(text: str) -> int:
    count = 0
    for line in text.splitlines():
        if is_step_line(line):
            count += 1
    return count


def is_step_line(line: str) -> bool:
    prefix, separator, _rest = line.partition(". ")
    return bool(separator) and prefix.isascii() and prefix.isdigit()


def migration_line(text: str) -> str | None:
    marker = f"{MIGRATION_VARIABLE}="
    for line in text.splitlines():
        if marker in line:
            return line.strip()
    return None


def last_paragraph(text: str) -> str:
    _earlier, separator, last = text.rpartition("\n\n")
    if not separator:
        return text
    return last


def announcement_report(
    verdict: MasterKeyBootVerdict, announcements: list[JsonValue]
) -> list[JsonValue]:
    match verdict:
        case UnsafeMasterKeyRefused() as refused:
            return [
                {"matches_rendered": text == f"\n{render_refusal(refused)}\n\n"}
                for text in announcements
            ]
        case SafeMasterKey() | UnsafeMasterKeyAllowed():
            return [{"matches_rendered": False} for _text in announcements]
