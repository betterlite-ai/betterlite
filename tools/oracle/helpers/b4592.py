import json
from collections.abc import Callable
from contextlib import ExitStack
from pathlib import Path
from tempfile import TemporaryDirectory
from typing import Final, TypedDict, cast
from unittest.mock import patch

from litellm.litellm_core_utils import cli_token_utils
from litellm.litellm_core_utils.cli_keyring import (
    KeyringDisabled,
    KeyringDiscardsWrites,
    KeyringNotInstalled,
    KeyringUnreachable,
    KeyringUnusable,
    SecretErase,
    SecretErased,
    SecretFound,
    SecretMissing,
    SecretRead,
    SecretStored,
    SecretStranded,
    SecretWrite,
)
from litellm.litellm_core_utils.cli_token_utils import (
    CliTokenRecord,
    CredentialNotCleared,
    CredentialNotRecorded,
    CredentialNotSaved,
    SecretClear,
    SecretSave,
    clear_cli_token,
    get_litellm_gateway_api_key,
    load_cli_token,
    save_cli_token,
)
from pydantic import JsonValue


class VaultInput(TypedDict, total=False):
    blob: str
    refuse: list[str]
    failure: str


class FailuresInput(TypedDict, total=False):
    stage: str
    refuse: list[str]


class Input(TypedDict, total=False):
    operation: str
    file: str
    record: dict[str, JsonValue]
    expected_base_url: str
    vault: VaultInput
    failures: FailuresInput


_FAILURES: Final[dict[str, KeyringUnusable]] = {
    "disabled": KeyringDisabled(),
    "not_installed": KeyringNotInstalled(),
    "unreachable": KeyringUnreachable(),
}

_SAVE_NAMES: Final[dict[type[object], str]] = {
    SecretStored: "secret_stored",
    KeyringDiscardsWrites: "keyring_discards_writes",
    KeyringDisabled: "keyring_disabled",
    KeyringNotInstalled: "keyring_not_installed",
    KeyringUnreachable: "keyring_unreachable",
    CredentialNotSaved: "credential_not_saved",
    CredentialNotRecorded: "credential_not_recorded",
}

_CLEAR_NAMES: Final[dict[type[object], str]] = {
    SecretErased: "secret_erased",
    SecretStranded: "secret_stranded",
    KeyringDisabled: "keyring_disabled",
    KeyringNotInstalled: "keyring_not_installed",
    KeyringUnreachable: "keyring_unreachable",
    CredentialNotCleared: "credential_not_cleared",
}

_ORIGINAL_READ: Final = cli_token_utils._read_token_file  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
_ORIGINAL_STAGE: Final = cli_token_utils._stage_token_file  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
_ORIGINAL_COMMIT: Final = cli_token_utils._commit_token_file  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
_ORIGINAL_WRITE: Final = cli_token_utils._write_token_file  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
_ORIGINAL_OVERWRITE: Final = cli_token_utils._overwrite_file_secret  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
_ORIGINAL_REMOVE: Final = cli_token_utils._remove_token_file  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]


class _Vault:
    def __init__(self, config: VaultInput, events: list[str]) -> None:
        self.blob: str | None = config.get("blob")
        self.refuse: frozenset[str] = frozenset(config.get("refuse", []))
        self.failure: KeyringUnusable = _FAILURES[config.get("failure", "unreachable")]
        self.events = events

    def read(self) -> SecretRead:
        self.events.append("vault_read")
        if "unavailable" in self.refuse:
            return self.failure
        if self.blob is None:
            return SecretMissing()
        return SecretFound(self.blob)

    def write(self, blob: str) -> SecretWrite:
        self.events.append("vault_write")
        if "unavailable" in self.refuse or "unwritable" in self.refuse:
            return self.failure
        if "discards" in self.refuse:
            return KeyringDiscardsWrites()
        self.blob = blob
        return SecretStored()

    def erase(self) -> SecretErase:
        self.events.append("vault_erase")
        if "unavailable" in self.refuse:
            return self.failure
        if "unerasable" in self.refuse:
            return SecretStranded() if self.blob is not None else SecretErased()
        self.blob = None
        return SecretErased()


class _Read:
    def __init__(self, events: list[str], original: Callable[[], CliTokenRecord | None]) -> None:
        self.events = events
        self.original = original

    def __call__(self) -> CliTokenRecord | None:
        self.events.append("read")
        return self.original()


class _Staged:
    def __init__(
        self,
        events: list[str],
        failure: str | None,
        path: Path,
        original: Callable[[CliTokenRecord], str | CredentialNotSaved],
    ) -> None:
        self.events = events
        self.failure = failure
        self.path = path
        self.original = original

    def __call__(self, record: CliTokenRecord) -> str | CredentialNotSaved:
        self.events.append("stage")
        failing = self.failure == "always" or (
            self.failure == "while_present" and self.path.exists()
        )
        if failing:
            return CredentialNotSaved("no space left on device")
        return self.original(record)


class _Committed:
    def __init__(
        self,
        events: list[str],
        *,
        refuse: bool,
        original: Callable[[str], bool],
    ) -> None:
        self.events = events
        self.refuse = refuse
        self.original = original

    def __call__(self, staged: str) -> bool:
        self.events.append("commit")
        if self.refuse:
            return False
        return self.original(staged)


class _Written:
    def __init__(
        self,
        events: list[str],
        *,
        refuse: bool,
        original: Callable[[CliTokenRecord], None],
    ) -> None:
        self.events = events
        self.refuse = refuse
        self.original = original

    def __call__(self, record: CliTokenRecord) -> None:
        self.events.append("write")
        if self.refuse:
            raise OSError("read-only file system")
        self.original(record)


class _Overwritten:
    def __init__(
        self,
        events: list[str],
        *,
        refuse: bool,
        original: Callable[[CliTokenRecord], bool],
    ) -> None:
        self.events = events
        self.refuse = refuse
        self.original = original

    def __call__(self, record: CliTokenRecord) -> bool:
        self.events.append("overwrite")
        if self.refuse:
            return False
        return self.original(record)


class _Removed:
    def __init__(
        self,
        events: list[str],
        *,
        refuse: bool,
        original: Callable[[], CredentialNotCleared | None],
    ) -> None:
        self.events = events
        self.refuse = refuse
        self.original = original

    def __call__(self) -> CredentialNotCleared | None:
        self.events.append("remove")
        if self.refuse:
            return CredentialNotCleared("the directory is read-only")
        return self.original()


def _patch(stack: ExitStack, failures: FailuresInput, events: list[str], path: Path) -> None:
    staged = _Staged(events, failures.get("stage"), path, _ORIGINAL_STAGE)
    stack.enter_context(patch.object(cli_token_utils, "_stage_token_file", new=staged))
    read = _Read(events, _ORIGINAL_READ)
    stack.enter_context(patch.object(cli_token_utils, "_read_token_file", new=read))
    refused = frozenset(failures.get("refuse", []))
    committed = _Committed(events, refuse="commit" in refused, original=_ORIGINAL_COMMIT)
    stack.enter_context(patch.object(cli_token_utils, "_commit_token_file", new=committed))
    written = _Written(events, refuse="write" in refused, original=_ORIGINAL_WRITE)
    stack.enter_context(patch.object(cli_token_utils, "_write_token_file", new=written))
    overwritten = _Overwritten(events, refuse="overwrite" in refused, original=_ORIGINAL_OVERWRITE)
    stack.enter_context(patch.object(cli_token_utils, "_overwrite_file_secret", new=overwritten))
    removed = _Removed(events, refuse="remove" in refused, original=_ORIGINAL_REMOVE)
    stack.enter_context(patch.object(cli_token_utils, "_remove_token_file", new=removed))


def _save_name(outcome: SecretSave) -> str:
    return _SAVE_NAMES[type(outcome)]


def _clear_name(outcome: SecretClear) -> str:
    return _CLEAR_NAMES[type(outcome)]


def _record_json(record: CliTokenRecord | None) -> JsonValue:
    if record is None:
        return None
    return cast(JsonValue, json.loads(record.model_dump_json(exclude_none=True)))


def _file_json(path: Path) -> JsonValue:
    if not path.exists():
        return None
    try:
        return _record_json(CliTokenRecord.model_validate_json(path.read_text()))
    except (OSError, ValueError):
        return None


def _vault_json(vault: _Vault) -> JsonValue:
    if vault.blob is None:
        return None
    try:
        return cast(JsonValue, json.loads(vault.blob))
    except json.JSONDecodeError:
        return None


def _answer(input_value: Input, vault: _Vault) -> dict[str, JsonValue]:
    operation = input_value.get("operation")
    if operation == "load":
        return {"loaded": _record_json(load_cli_token(vault=vault))}
    if operation == "save":
        record = input_value.get("record")
        if record is None:
            raise ValueError("save needs a record")
        stored = save_cli_token(CliTokenRecord.model_validate(record), vault=vault)
        return {"outcome": _save_name(stored)}
    if operation == "clear":
        return {"outcome": _clear_name(clear_cli_token(vault=vault))}
    if operation == "gateway_key":
        key = get_litellm_gateway_api_key(
            expected_base_url=input_value.get("expected_base_url"),
            vault=vault,
        )
        return {"key": key}
    raise ValueError("unknown credential lifecycle operation")


def run(input_value: Input) -> JsonValue:
    with TemporaryDirectory() as root, patch.dict("os.environ", {"HOME": root}):
        path = Path(root) / ".litellm" / "token.json"
        fixture = input_value.get("file")
        if fixture is not None:
            path.parent.mkdir(parents=True, exist_ok=True)
            _ = path.write_text(fixture)
        events: list[str] = []
        vault = _Vault(input_value.get("vault", {}), events)
        with ExitStack() as stack:
            _patch(stack, input_value.get("failures", {}), events, path)
            answer = _answer(input_value, vault)
        return cast(
            JsonValue,
            {
                "events": events,
                "file": _file_json(path),
                "vault": _vault_json(vault),
                **answer,
            },
        )
