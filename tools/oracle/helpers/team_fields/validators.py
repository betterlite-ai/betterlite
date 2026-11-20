import asyncio
from typing import cast

from litellm.proxy.management_helpers.team_metadata_validation import (
    TeamMetadataValidationPayload,
    TeamMetadataValidationResult,
    TeamMetadataValidator,
)
from pydantic import JsonValue

from .inputs import PAYLOAD, PayloadDict, ValidatorKind


class Recording:
    def __init__(
        self,
        result: TeamMetadataValidationResult,
        *,
        error: Exception | None = None,
        sleep: float = 0.0,
    ) -> None:
        self.payloads: list[PayloadDict] = []
        self._result = result
        self._error = error
        self._sleep = sleep

    async def __call__(
        self, payload: TeamMetadataValidationPayload
    ) -> TeamMetadataValidationResult:
        self.payloads.append(PAYLOAD.validate_json(payload.model_dump_json()))
        if self._error is not None:
            raise self._error
        if self._sleep:
            await asyncio.sleep(self._sleep)
        return self._result


class SyncRecording:
    def __init__(self) -> None:
        self.payloads: list[PayloadDict] = []

    def __call__(self, _payload: TeamMetadataValidationPayload) -> TeamMetadataValidationResult:
        return TeamMetadataValidationResult(valid=True)


class Malformed:
    def __init__(self, result: JsonValue) -> None:
        self.payloads: list[PayloadDict] = []
        self._result = result

    async def __call__(self, payload: TeamMetadataValidationPayload) -> JsonValue:
        self.payloads.append(PAYLOAD.validate_json(payload.model_dump_json()))
        return self._result


class Marker:
    async def __call__(
        self, _payload: TeamMetadataValidationPayload
    ) -> TeamMetadataValidationResult:
        return TeamMetadataValidationResult(valid=True)


VALIDATOR_A = Marker()
VALIDATOR_B = Marker()


async def _accept_function(  # ast-grep-ignore: no-one-line-functions
    _payload: TeamMetadataValidationPayload,
) -> TeamMetadataValidationResult:
    return TeamMetadataValidationResult(valid=True)


def _result_for(kind: ValidatorKind) -> TeamMetadataValidationResult:
    match kind:
        case "reject":
            return TeamMetadataValidationResult(valid=False, error_message="alias must be set")
        case "reject_empty":
            return TeamMetadataValidationResult(valid=False, error_message="")
        case _:
            return TeamMetadataValidationResult(valid=True)


def build_validator(
    kind: ValidatorKind,
) -> tuple[TeamMetadataValidator | None, list[PayloadDict]]:
    if kind == "absent":
        return None, []
    if kind == "function":
        return _accept_function, []
    if kind == "sync":
        sync = SyncRecording()
        return cast("TeamMetadataValidator", sync), sync.payloads  # cast-ok: the async check rejects a plain callable before it runs  # noqa: E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_helpers/team_metadata_validation.py#L118-L124
    recorder: Recording | Malformed
    if kind in {"malformed_type", "malformed_object"}:
        raw: JsonValue = {"valid": "yes"} if kind == "malformed_type" else "ok"
        recorder = Malformed(raw)
    else:
        recorder = Recording(
            _result_for(kind),
            error=ValueError("database exploded secret-token-123") if kind == "raises" else None,
            sleep=0.2 if kind == "sleep" else 0.0,
        )
    return cast("TeamMetadataValidator", recorder), recorder.payloads  # cast-ok: a malformed result is exactly what the guarded model_validate must reject  # noqa: E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_helpers/team_metadata_validation.py#L126-L133
