from ._bridge import get_gateway_api_key
from .chat import ChatClient
from .credentials import CredentialsManagementClient
from .http_client import HTTPClient
from .keys import KeysManagementClient
from .model_groups import ModelGroupsManagementClient
from .models import ModelsManagementClient
from .teams import TeamsManagementClient


class Client:
    def __init__(
        self,
        base_url: str,
        api_key: str | None = None,
        timeout: int = 30,
    ) -> None:
        self._base_url = base_url.rstrip("/")
        self._api_key = api_key or get_gateway_api_key(expected_base_url=self._base_url)

        self.http = HTTPClient(base_url=base_url, api_key=self._api_key, timeout=timeout)
        self.models = ModelsManagementClient(
            base_url=self._base_url, api_key=self._api_key, timeout=timeout
        )
        self.model_groups = ModelGroupsManagementClient(
            base_url=self._base_url, api_key=self._api_key, timeout=timeout
        )
        # ChatClient keeps its own 600-second default, since a completion can take minutes.
        self.chat = ChatClient(base_url=self._base_url, api_key=self._api_key)
        self.keys = KeysManagementClient(
            base_url=self._base_url, api_key=self._api_key, timeout=timeout
        )
        self.credentials = CredentialsManagementClient(
            base_url=self._base_url, api_key=self._api_key, timeout=timeout
        )
        self.teams = TeamsManagementClient(
            base_url=self._base_url, api_key=self._api_key, timeout=timeout
        )
