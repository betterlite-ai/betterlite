#![cfg(test)]

mod http_server;
mod support;

mod secret_dispatch;
mod secret_dispatch_aws_kms;
mod secret_dispatch_aws_secrets;
mod secret_dispatch_azure;
mod secret_dispatch_base64;
mod secret_dispatch_custom;
mod secret_dispatch_cyberark;
mod secret_dispatch_google_kms;
mod secret_dispatch_google_secrets;
mod secret_dispatch_infisical;
mod secret_dispatch_local;
mod secret_dispatch_missing_value;
mod secret_dispatch_protocols;
mod secret_dispatch_vault;
