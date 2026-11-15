from enum import StrEnum


class AzureCredentialType(StrEnum):
    ClientSecretCredential = "ClientSecretCredential"
    ManagedIdentityCredential = "ManagedIdentityCredential"
    CertificateCredential = "CertificateCredential"
    DefaultAzureCredential = "DefaultAzureCredential"
    DeploymentIdentityCredential = "DeploymentIdentityCredential"
