from collections.abc import Mapping
from typing import Final

from typing_extensions import TypedDict

from betterlite.types.llms.openai.core.files import CreateFileRequest

# Litellm's six plausible labels for a .jsonl batch upload; purpose == "batch" is the real signal
_BATCH_JSONL_CONTENT_TYPES: Final = frozenset(
    {
        "application/jsonl",
        "application/json",
        "application/octet-stream",
        "application/x-ndjson",
        "application/x-jsonlines",
        "text/plain",
    }
)


class ExtractedFileData(TypedDict):
    filename: str | None
    content: bytes
    content_type: str | None
    headers: Mapping[str, str]


class FilesAPIUtils:
    @staticmethod
    def is_batch_jsonl_file(
        create_file_data: CreateFileRequest, extracted_file_data: ExtractedFileData
    ) -> bool:
        return FilesAPIUtils.is_batch_jsonl_request(
            create_file_data, extracted_file_data["content_type"],
        )

    @staticmethod
    def is_batch_jsonl_request(
        create_file_data: CreateFileRequest, content_type: str | None
    ) -> bool:
        return (
            create_file_data["purpose"] == "batch"
            and FilesAPIUtils.valid_content_type(content_type)
        )

    @staticmethod
    # ast-grep-ignore: no-one-line-functions
    def valid_content_type(content_type: str | None) -> bool:
        return content_type in _BATCH_JSONL_CONTENT_TYPES
