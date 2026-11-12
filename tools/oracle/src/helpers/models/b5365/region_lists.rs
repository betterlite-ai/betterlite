use betterlite_models::deployment_regions::ProviderRegionLists;
use serde_json::{Value, json};

struct Listed {
    provider: &'static str,
    eu: &'static [&'static str],
    us: &'static [&'static str],
}

// copied from the provider configs' get_eu_regions and get_us_regions, and every answer checks them against LiteLLM's
const LISTED: [Listed; 4] = [
    Listed {
        provider: "azure",
        eu: &["europe", "sweden", "switzerland", "france", "uk"],
        us: &[
            "us",
            "eastus",
            "eastus2",
            "eastus2euap",
            "eastus3",
            "southcentralus",
            "westus",
            "westus2",
            "westus3",
            "westus4",
        ],
    },
    Listed {
        provider: "vertex_ai",
        eu: &[
            "europe-central2",
            "europe-north1",
            "europe-southwest1",
            "europe-west1",
            "europe-west2",
            "europe-west3",
            "europe-west4",
            "europe-west6",
            "europe-west8",
            "europe-west9",
        ],
        us: &[
            "us-central1",
            "us-east1",
            "us-east4",
            "us-east5",
            "us-south1",
            "us-west1",
            "us-west4",
            "us-west5",
        ],
    },
    Listed {
        provider: "bedrock",
        eu: &[
            "eu-west-1",
            "eu-west-2",
            "eu-west-3",
            "eu-central-1",
            "eu-central-2",
            "eu-south-1",
            "eu-south-2",
            "eu-north-1",
        ],
        us: &[
            "us-east-1",
            "us-east-2",
            "us-west-1",
            "us-west-2",
            "us-gov-east-1",
            "us-gov-west-1",
        ],
    },
    Listed {
        provider: "watsonx",
        eu: &["eu-de", "eu-gb"],
        us: &["us-south"],
    },
];

pub(super) fn provider_lists() -> ProviderRegionLists {
    let mut lists = ProviderRegionLists::default();
    for listed in &LISTED {
        let target = match listed.provider {
            "azure" => &mut lists.azure,
            "vertex_ai" => &mut lists.vertex_ai,
            "bedrock" => &mut lists.bedrock,
            _ => &mut lists.watsonx,
        };
        target.eu = listed
            .eu
            .iter()
            .map(|region| (*region).to_owned())
            .collect();
        target.us = listed
            .us
            .iter()
            .map(|region| (*region).to_owned())
            .collect();
    }
    lists
}

pub(super) fn lists_answer() -> Value {
    Value::Object(
        LISTED
            .iter()
            .map(|listed| {
                (
                    listed.provider.to_owned(),
                    json!({"eu": listed.eu, "us": listed.us}),
                )
            })
            .collect(),
    )
}
