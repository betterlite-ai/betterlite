from betterlite.types.utils.base import ResponseObject


class AzureContentSafetyConfigModel(ResponseObject):
    api_key: str | None = None
    api_base: str | None = None
    api_version: str | None = "2024-09-01"
