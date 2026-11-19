import os

from litellm.proxy.ui_crud_endpoints import latest_release_endpoints


def set_release_url() -> None:
    destination = os.environ["RELEASE_ORACLE_URL"]
    object.__setattr__(latest_release_endpoints, "LATEST_RELEASE_URL", destination)
