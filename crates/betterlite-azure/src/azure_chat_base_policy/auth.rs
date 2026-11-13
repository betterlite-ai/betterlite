use betterlite_prompts::arguments::json::Object;

pub const SPECIAL_AUTH_PARAMS: [(&str, &str); 1] = [("token", "azure_ad_token")];

pub const EU_REGIONS: [&str; 5] = ["europe", "sweden", "switzerland", "france", "uk"];

pub const US_REGIONS: [&str; 10] = [
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
];

pub fn map_special_auth_params(non_default: &Object, optional: &mut Object) {
    for (param, mapped) in SPECIAL_AUTH_PARAMS {
        if let Some(value) = non_default.get(param) {
            optional.insert(mapped.to_owned(), value.clone());
        }
    }
}
