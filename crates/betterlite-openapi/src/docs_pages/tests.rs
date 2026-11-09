#![cfg(test)]

use std::fs;

use rstest::rstest;

use crate::docs_pages::{
    SwaggerPageInputs, oauth2_redirect_page, redoc_page, swagger_asset, swagger_ui_page,
};

const SWAGGER_ROOT: &str = concat!(
    "\n    <!DOCTYPE html>\n    <html>\n    <head>\n    <meta name=\"viewport\" content=\"width=device-width, initial-scale=1.0\">\n    <link type=\"text/css\" rel=\"stylesheet\" href=\"/swagger/swagger-ui.css\">\n",
    "    <link rel=\"shortcut icon\" href=\"/swagger/favicon.png\">\n    <title>LiteLLM API - Swagger UI</title>\n    </head>\n    <body>\n    <div id=\"swagger-ui\">\n    </div>\n    <script src=\"/swagger/swagger-ui-bundle.js\"></script>\n",
    "    <!-- `SwaggerUIBundle` is now available on the page -->\n    <script>\n    const ui = SwaggerUIBundle({tagsSorter:\"alpha\",\n        url: '/openapi.json',\n    \"dom_id\": \"#swagger-ui\",\n\"layout\": \"BaseLayout\",\n\"deepLinking\": true,\n",
    "\"showExtensions\": true,\n\"showCommonExtensions\": true,\noauth2RedirectUrl: window.location.origin + '/docs/oauth2-redirect',\n    presets: [\n        SwaggerUIBundle.presets.apis,\n        SwaggerUIBundle.SwaggerUIStandalonePreset\n        ],\n",
    "    })\n    </script>\n    </body>\n    </html>\n    ",
);

const SWAGGER_ROOT_PATH: &str = concat!(
    "\n    <!DOCTYPE html>\n    <html>\n    <head>\n    <meta name=\"viewport\" content=\"width=device-width, initial-scale=1.0\">\n    <link type=\"text/css\" rel=\"stylesheet\" href=\"/api/swagger/swagger-ui.css\">\n",
    "    <link rel=\"shortcut icon\" href=\"/api/swagger/favicon.png\">\n    <title>LiteLLM API - Swagger UI</title>\n    </head>\n    <body>\n    <div id=\"swagger-ui\">\n    </div>\n    <script src=\"/api/swagger/swagger-ui-bundle.js\"></script>\n",
    "    <!-- `SwaggerUIBundle` is now available on the page -->\n    <script>\n    const ui = SwaggerUIBundle({tagsSorter:\"alpha\",\n        url: '/api/openapi.json',\n    \"dom_id\": \"#swagger-ui\",\n\"layout\": \"BaseLayout\",\n",
    "\"deepLinking\": true,\n\"showExtensions\": true,\n\"showCommonExtensions\": true,\noauth2RedirectUrl: window.location.origin + '/api/docs/oauth2-redirect',\n    presets: [\n        SwaggerUIBundle.presets.apis,\n",
    "        SwaggerUIBundle.SwaggerUIStandalonePreset\n        ],\n    })\n    </script>\n    </body>\n    </html>\n    ",
);

const SWAGGER_NO_OAUTH: &str = concat!(
    "\n    <!DOCTYPE html>\n    <html>\n    <head>\n    <meta name=\"viewport\" content=\"width=device-width, initial-scale=1.0\">\n    <link type=\"text/css\" rel=\"stylesheet\" href=\"/swagger/swagger-ui.css\">\n",
    "    <link rel=\"shortcut icon\" href=\"/swagger/favicon.png\">\n    <title>LiteLLM API - Swagger UI</title>\n    </head>\n    <body>\n    <div id=\"swagger-ui\">\n    </div>\n    <script src=\"/swagger/swagger-ui-bundle.js\"></script>\n",
    "    <!-- `SwaggerUIBundle` is now available on the page -->\n    <script>\n    const ui = SwaggerUIBundle({tagsSorter:\"alpha\",\n        url: '/openapi.json',\n    \"dom_id\": \"#swagger-ui\",\n\"layout\": \"BaseLayout\",\n\"deepLinking\": true,\n",
    "\"showExtensions\": true,\n\"showCommonExtensions\": true,\n\n    presets: [\n        SwaggerUIBundle.presets.apis,\n        SwaggerUIBundle.SwaggerUIStandalonePreset\n        ],\n    })\n    </script>\n    </body>\n    </html>\n    ",
);

const REDOC_ROOT: &str = concat!(
    "\n    <!DOCTYPE html>\n    <html>\n    <head>\n    <title>LiteLLM API - ReDoc</title>\n    <!-- needed for adaptive design -->\n    <meta charset=\"utf-8\"/>\n    <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n    \n",
    "    <link href=\"https://fonts.googleapis.com/css?family=Montserrat:300,400,700|Roboto:300,400,700\" rel=\"stylesheet\">\n    \n    <link rel=\"shortcut icon\" href=\"https://fastapi.tiangolo.com/img/favicon.png\">\n    <!--\n",
    "    ReDoc doesn't change outer page styles\n    -->\n    <style>\n      body {\n        margin: 0;\n        padding: 0;\n      }\n    </style>\n    </head>\n    <body>\n    <noscript>\n",
    "        ReDoc requires Javascript to function. Please enable it to browse the documentation.\n    </noscript>\n    <redoc spec-url=\"/openapi.json\"></redoc>\n",
    "    <script src=\"https://cdn.jsdelivr.net/npm/redoc@2/bundles/redoc.standalone.js\"> </script>\n    </body>\n    </html>\n    ",
);

const REDOC_ROOT_PATH: &str = concat!(
    "\n    <!DOCTYPE html>\n    <html>\n    <head>\n    <title>LiteLLM API - ReDoc</title>\n    <!-- needed for adaptive design -->\n    <meta charset=\"utf-8\"/>\n    <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n    \n",
    "    <link href=\"https://fonts.googleapis.com/css?family=Montserrat:300,400,700|Roboto:300,400,700\" rel=\"stylesheet\">\n    \n    <link rel=\"shortcut icon\" href=\"https://fastapi.tiangolo.com/img/favicon.png\">\n    <!--\n",
    "    ReDoc doesn't change outer page styles\n    -->\n    <style>\n      body {\n        margin: 0;\n        padding: 0;\n      }\n    </style>\n    </head>\n    <body>\n    <noscript>\n",
    "        ReDoc requires Javascript to function. Please enable it to browse the documentation.\n    </noscript>\n    <redoc spec-url=\"/api/openapi.json\"></redoc>\n",
    "    <script src=\"https://cdn.jsdelivr.net/npm/redoc@2/bundles/redoc.standalone.js\"> </script>\n    </body>\n    </html>\n    ",
);

const OAUTH2_REDIRECT: &str = concat!(
    "\n    <!doctype html>\n    <html lang=\"en-US\">\n    <head>\n        <title>Swagger UI: OAuth2 Redirect</title>\n    </head>\n    <body>\n    <script>\n        'use strict';\n        function run () {\n",
    "            var oauth2 = window.opener.swaggerUIRedirectOauth2;\n            var sentState = oauth2.state;\n            var redirectUrl = oauth2.redirectUrl;\n            var isValid, qp, arr;\n\n",
    "            if (/code|token|error/.test(window.location.hash)) {\n                qp = window.location.hash.substring(1).replace('?', '&');\n            } else {\n                qp = location.search.substring(1);\n            }\n\n",
    "            arr = qp.split(\"&\");\n            arr.forEach(function (v,i,_arr) { _arr[i] = '\"' + v.replace('=', '\":\"') + '\"';});\n            qp = qp ? JSON.parse('{' + arr.join() + '}',\n                    function (key, value) {\n",
    "                        return key === \"\" ? value : decodeURIComponent(value);\n                    }\n            ) : {};\n\n            isValid = qp.state === sentState;\n\n            if ((\n",
    "              oauth2.auth.schema.get(\"flow\") === \"accessCode\" ||\n              oauth2.auth.schema.get(\"flow\") === \"authorizationCode\" ||\n              oauth2.auth.schema.get(\"flow\") === \"authorization_code\"\n",
    "            ) && !oauth2.auth.code) {\n                if (!isValid) {\n                    oauth2.errCb({\n                        authId: oauth2.auth.name,\n                        source: \"auth\",\n                        level: \"warning\",\n",
    "                        message: \"Authorization may be unsafe, passed state was changed in server. The passed state wasn't returned from auth server.\"\n                    });\n                }\n\n                if (qp.code) {\n",
    "                    delete oauth2.state;\n                    oauth2.auth.code = qp.code;\n                    oauth2.callback({auth: oauth2.auth, redirectUrl: redirectUrl});\n                } else {\n                    let oauthErrorMsg;\n",
    "                    if (qp.error) {\n                        oauthErrorMsg = \"[\"+qp.error+\"]: \" +\n                            (qp.error_description ? qp.error_description+ \". \" : \"no accessCode received from the server. \") +\n",
    "                            (qp.error_uri ? \"More info: \"+qp.error_uri : \"\");\n                    }\n\n                    oauth2.errCb({\n                        authId: oauth2.auth.name,\n                        source: \"auth\",\n",
    "                        level: \"error\",\n                        message: oauthErrorMsg || \"[Authorization failed]: no accessCode received from the server.\"\n                    });\n                }\n            } else {\n",
    "                oauth2.callback({auth: oauth2.auth, token: qp, isValid: isValid, redirectUrl: redirectUrl});\n            }\n            window.close();\n        }\n\n        if (document.readyState !== 'loading') {\n            run();\n",
    "        } else {\n            document.addEventListener('DOMContentLoaded', function () {\n                run();\n            });\n        }\n    </script>\n    </body>\n    </html>\n        ",
);

fn inputs(
    title: &str,
    openapi_url: &str,
    oauth2_redirect_url: Option<&str>,
    swagger_base: &str,
) -> SwaggerPageInputs {
    SwaggerPageInputs {
        title: title.to_owned(),
        openapi_url: openapi_url.to_owned(),
        oauth2_redirect_url: oauth2_redirect_url.map(str::to_owned),
        swagger_base: swagger_base.to_owned(),
    }
}

#[test]
fn swagger_page_matches_fastapi_without_root_path() {
    let page = swagger_ui_page(&inputs(
        "LiteLLM API - Swagger UI",
        "/openapi.json",
        Some("/docs/oauth2-redirect"),
        "/swagger",
    ));
    assert_eq!(page, SWAGGER_ROOT);
}

#[test]
fn swagger_page_matches_fastapi_with_root_path() {
    let page = swagger_ui_page(&inputs(
        "LiteLLM API - Swagger UI",
        "/api/openapi.json",
        Some("/api/docs/oauth2-redirect"),
        "/api/swagger",
    ));
    assert_eq!(page, SWAGGER_ROOT_PATH);
}

#[test]
fn swagger_page_without_oauth2_redirect_matches_fastapi() {
    let page = swagger_ui_page(&inputs(
        "LiteLLM API - Swagger UI",
        "/openapi.json",
        None,
        "/swagger",
    ));
    assert_eq!(page, SWAGGER_NO_OAUTH);
}

#[test]
fn redoc_page_matches_fastapi_without_root_path() {
    assert_eq!(
        redoc_page("LiteLLM API - ReDoc", "/openapi.json"),
        REDOC_ROOT
    );
}

#[test]
fn redoc_page_matches_fastapi_with_root_path() {
    assert_eq!(
        redoc_page("LiteLLM API - ReDoc", "/api/openapi.json"),
        REDOC_ROOT_PATH
    );
}

#[test]
fn oauth2_redirect_page_matches_fastapi() {
    assert_eq!(oauth2_redirect_page(), OAUTH2_REDIRECT);
}

#[rstest]
#[case::bundle("swagger-ui-bundle.js", "text/javascript")]
#[case::stylesheet("swagger-ui.css", "text/css")]
#[case::png("favicon.png", "image/png")]
#[case::ico("favicon.ico", "image/vnd.microsoft.icon")]
fn swagger_asset_returns_the_embedded_file(#[case] name: &str, #[case] media_type: &str) {
    let asset = swagger_asset(name).unwrap();
    assert_eq!(asset.content_type, media_type);
    let served = fs::read(format!(
        "{}/assets/swagger/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    assert_eq!(asset.bytes, served);
}

#[rstest]
#[case::parent("../swagger-ui.css")]
#[case::nested("a/swagger-ui.css")]
#[case::empty("")]
#[case::uppercase("SWAGGER-UI.CSS")]
#[case::trailing_space("swagger-ui.css ")]
#[case::dot(".")]
#[case::dotdot("..")]
fn swagger_asset_rejects_other_names(#[case] name: &str) {
    assert!(swagger_asset(name).is_none());
}
