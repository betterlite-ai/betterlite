use betterlite_config::sections::user_url_policy::Settings;
use betterlite_config::tls::Verification;
use betterlite_core::rag::ingestion::upload::HttpUrlFetch;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::user_url::fetch::UserUrlFetcher;
use betterlite_http::user_url::validate::{SystemResolver, UrlValidator};

mod tests;

pub type UrlFetch = HttpUrlFetch<SystemResolver>;

// B5082's fetch, with the RAG download bound, under the user URL policy the checks read
pub fn ssrf_checked(client: ProxiedClient, settings: &Settings) -> UrlFetch {
    UrlFetch {
        client,
        urls: UserUrlFetcher {
            validator: UrlValidator::new(
                SystemResolver,
                &settings.user_url_allowed_hosts,
                &Verification::Enabled,
            ),
            validation: settings.user_url_validation,
        },
    }
}
