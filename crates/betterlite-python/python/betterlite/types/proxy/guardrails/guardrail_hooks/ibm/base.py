from betterlite.types.utils.base import ResponseObject


class IBMGuardrailsBaseConfigModel(ResponseObject):
    auth_token: str | None = None
    base_url: str | None = None
    detector_id: str | None = None
    is_detector_server: bool | None = True
    verify_ssl: bool | None = True
