from typing import TypeAlias, cast
from unittest.mock import patch
from uuid import UUID

from litellm.proxy.rag_endpoints.upload_security import (
    EicarTestMalwareScanner,
    ScanResult,
    ScanVerdict,
    SecuredUpload,
    validate_upload,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class _Scanner:
    def __init__(self, verdict: ScanVerdict, signature: str | None) -> None:
        self.verdict = verdict
        self.signature = signature

    def scan(self, content: bytes) -> ScanResult:
        del content
        return ScanResult(verdict=self.verdict, signature=self.signature)


def run(input_value: dict[str, Json]) -> Json:
    content = bytes.fromhex(cast(str, input_value["content_hex"]))
    scanner_name = cast(str, input_value.get("scanner", "clean"))
    max_size = cast(int, input_value.get("max_size_bytes", 512 * 1024 * 1024))
    if scanner_name == "eicar":
        scanner = EicarTestMalwareScanner()
    else:
        verdict = {
            "clean": ScanVerdict.CLEAN,
            "infected": ScanVerdict.INFECTED,
            "error": ScanVerdict.ERROR,
        }.get(scanner_name)
        if verdict is None:
            raise ValueError(f"unknown scanner {scanner_name}")
        scanner = _Scanner(verdict, cast(str | None, input_value.get("signature")))
    identifier = UUID(cast(str, input_value.get("uuid", "550e8400-e29b-41d4-a716-446655440000")))
    with patch(
        "litellm.proxy.rag_endpoints.upload_security.uuid.uuid4",
        return_value=identifier,
    ):
        result = validate_upload(content=content, scanner=scanner, max_size_bytes=max_size)
    if isinstance(result, SecuredUpload):
        return {
            "status": "secured",
            "safe_filename": result.safe_filename,
            "content_type": result.content_type,
            "detected_format": result.detected_format.value,
            "size_bytes": result.size_bytes,
        }
    return {
        "status": "rejected",
        "reason": result.reason.value,
        "message": result.message,
    }
