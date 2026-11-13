mod tests;

#[expect(
    clippy::large_include_file,
    reason = "LiteLLM's pinned swagger-ui bundle is embedded byte-for-byte"
)]
const SWAGGER_UI_BUNDLE: &[u8] = include_bytes!("../assets/swagger/swagger-ui-bundle.js");
const SWAGGER_UI_CSS: &[u8] = include_bytes!("../assets/swagger/swagger-ui.css");
const FAVICON_PNG: &[u8] = include_bytes!("../assets/swagger/favicon.png");
const FAVICON_ICO: &[u8] = include_bytes!("../assets/swagger/favicon.ico");

pub struct SwaggerAsset {
    pub bytes: &'static [u8],
    pub content_type: &'static str,
}

pub struct SwaggerPageInputs {
    pub title: String,
    pub openapi_url: String,
    pub oauth2_redirect_url: Option<String>,
    pub swagger_base: String,
}

#[must_use]
pub fn swagger_ui_page(inputs: &SwaggerPageInputs) -> String {
    let scripts = format!("{}/swagger-ui-bundle.js", inputs.swagger_base);
    let stylesheet = format!("{}/swagger-ui.css", inputs.swagger_base);
    let favicon = format!("{}/favicon.png", inputs.swagger_base);
    let oauth2_redirect = inputs
        .oauth2_redirect_url
        .as_ref()
        .map_or_else(String::new, |url| {
            format!("oauth2RedirectUrl: window.location.origin + '{url}',")
        });
    format!(
        concat!(
            "\n    <!DOCTYPE html>\n    <html>\n    <head>\n    <meta name=\"viewport\" content=\"width=device-width, initial-scale=1.0\">\n    <link type=\"text/css\" rel=\"stylesheet\" href=\"{css_url}\">\n",
            "    <link rel=\"shortcut icon\" href=\"{favicon_url}\">\n    <title>{title}</title>\n    </head>\n    <body>\n    <div id=\"swagger-ui\">\n    </div>\n    <script src=\"{js_url}\"></script>\n",
            "    <!-- `SwaggerUIBundle` is now available on the page -->\n    <script>\n    const ui = SwaggerUIBundle({{tagsSorter:\"alpha\",\n        url: '{openapi_url}',\n    \"dom_id\": \"#swagger-ui\",\n\"layout\": \"BaseLayout\",\n\"deepLinking\": true,\n",
            "\"showExtensions\": true,\n\"showCommonExtensions\": true,\n{oauth2_redirect}\n    presets: [\n        SwaggerUIBundle.presets.apis,\n        SwaggerUIBundle.SwaggerUIStandalonePreset\n        ],\n    }})\n    </script>\n    </body>\n    </html>\n",
            "    ",
        ),
        css_url = stylesheet,
        favicon_url = favicon,
        title = inputs.title,
        js_url = scripts,
        openapi_url = inputs.openapi_url,
        oauth2_redirect = oauth2_redirect,
    )
}

#[must_use]
pub fn redoc_page(title: &str, openapi_url: &str) -> String {
    format!(
        concat!(
            "\n    <!DOCTYPE html>\n    <html>\n    <head>\n    <title>{title}</title>\n    <!-- needed for adaptive design -->\n    <meta charset=\"utf-8\"/>\n    <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n    \n",
            "    <link href=\"https://fonts.googleapis.com/css?family=Montserrat:300,400,700|Roboto:300,400,700\" rel=\"stylesheet\">\n    \n    <link rel=\"shortcut icon\" href=\"https://fastapi.tiangolo.com/img/favicon.png\">\n    <!--\n",
            "    ReDoc doesn't change outer page styles\n    -->\n    <style>\n      body {{\n        margin: 0;\n        padding: 0;\n      }}\n    </style>\n    </head>\n    <body>\n    <noscript>\n",
            "        ReDoc requires Javascript to function. Please enable it to browse the documentation.\n    </noscript>\n    <redoc spec-url=\"{openapi_url}\"></redoc>\n",
            "    <script src=\"https://cdn.jsdelivr.net/npm/redoc@2/bundles/redoc.standalone.js\"> </script>\n    </body>\n    </html>\n    ",
        ),
        title = title,
        openapi_url = openapi_url,
    )
}

#[must_use]
pub const fn oauth2_redirect_page() -> &'static str {
    concat!(
        "\n    <!doctype html>\n    <html lang=\"en-US\">\n    <head>\n        <title>Swagger UI: OAuth2 Redirect</title>\n    </head>\n    <body>\n    <script>\n        'use strict';\n        function run () {\n            var oauth2 = window.opener.swaggerUIRedirectOauth2;\n",
        "            var sentState = oauth2.state;\n            var redirectUrl = oauth2.redirectUrl;\n            var isValid, qp, arr;\n\n            if (/code|token|error/.test(window.location.hash)) {\n                qp = window.location.hash.substring(1).replace('?', '&');\n",
        "            } else {\n                qp = location.search.substring(1);\n            }\n\n            arr = qp.split(\"&\");\n            arr.forEach(function (v,i,_arr) { _arr[i] = '\"' + v.replace('=', '\":\"') + '\"';});\n            qp = qp ? JSON.parse('{' + arr.join() + '}',\n",
        "                    function (key, value) {\n                        return key === \"\" ? value : decodeURIComponent(value);\n                    }\n            ) : {};\n\n            isValid = qp.state === sentState;\n\n            if ((\n",
        "              oauth2.auth.schema.get(\"flow\") === \"accessCode\" ||\n              oauth2.auth.schema.get(\"flow\") === \"authorizationCode\" ||\n              oauth2.auth.schema.get(\"flow\") === \"authorization_code\"\n            ) && !oauth2.auth.code) {\n",
        "                if (!isValid) {\n                    oauth2.errCb({\n                        authId: oauth2.auth.name,\n                        source: \"auth\",\n                        level: \"warning\",\n                        message: \"Authorization may be unsafe, passed state was changed in server. The passed state wasn't returned from auth server.\"\n",
        "                    });\n                }\n\n                if (qp.code) {\n                    delete oauth2.state;\n                    oauth2.auth.code = qp.code;\n                    oauth2.callback({auth: oauth2.auth, redirectUrl: redirectUrl});\n",
        "                } else {\n                    let oauthErrorMsg;\n                    if (qp.error) {\n                        oauthErrorMsg = \"[\"+qp.error+\"]: \" +\n                            (qp.error_description ? qp.error_description+ \". \" : \"no accessCode received from the server. \") +\n",
        "                            (qp.error_uri ? \"More info: \"+qp.error_uri : \"\");\n                    }\n\n                    oauth2.errCb({\n                        authId: oauth2.auth.name,\n                        source: \"auth\",\n",
        "                        level: \"error\",\n                        message: oauthErrorMsg || \"[Authorization failed]: no accessCode received from the server.\"\n                    });\n                }\n            } else {\n                oauth2.callback({auth: oauth2.auth, token: qp, isValid: isValid, redirectUrl: redirectUrl});\n",
        "            }\n            window.close();\n        }\n\n        if (document.readyState !== 'loading') {\n            run();\n        } else {\n            document.addEventListener('DOMContentLoaded', function () {\n                run();\n",
        "            });\n        }\n    </script>\n    </body>\n    </html>\n        ",
    )
}

#[must_use]
pub fn swagger_asset(name: &str) -> Option<SwaggerAsset> {
    match name {
        "swagger-ui-bundle.js" => Some(SwaggerAsset {
            bytes: SWAGGER_UI_BUNDLE,
            content_type: "text/javascript",
        }),
        "swagger-ui.css" => Some(SwaggerAsset {
            bytes: SWAGGER_UI_CSS,
            content_type: "text/css",
        }),
        "favicon.png" => Some(SwaggerAsset {
            bytes: FAVICON_PNG,
            content_type: "image/png",
        }),
        "favicon.ico" => Some(SwaggerAsset {
            bytes: FAVICON_ICO,
            content_type: "image/vnd.microsoft.icon",
        }),
        _ => None,
    }
}
