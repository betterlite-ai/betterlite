#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizePage {
    pub server_name: String,
    pub server_initial: String,
    pub client_id: String,
    pub redirect_uri: String,
    pub code_challenge: String,
    pub code_challenge_method: String,
    pub state: String,
    pub server_id: String,
    pub access_items: Vec<String>,
    pub help_url: String,
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

fn access_section(page: &AuthorizePage) -> String {
    let mut items = String::new();
    for item in &page.access_items {
        items.push_str("<div class=\"access-item\"><span class=\"check\">&#10003;</span>");
        items.push_str(&escape_html(item));
        items.push_str("</div>");
    }
    if items.is_empty() {
        String::new()
    } else {
        format!(
            "<div class=\"access-box\"><div class=\"access-header\">Requested Access</div>{items}</div>"
        )
    }
}

fn safe_help_link(help_url: &str) -> String {
    if help_url.is_empty() {
        return String::new();
    }
    let active = url::Url::parse(help_url).map_or_else(
        |_| {
            url::Url::parse("https://betterlite.invalid/")
                .and_then(|base| base.join(help_url))
                .is_ok_and(|url| matches!(url.scheme(), "javascript" | "data" | "vbscript"))
        },
        |url| matches!(url.scheme(), "javascript" | "data" | "vbscript"),
    );
    if active {
        String::new()
    } else {
        format!(
            "<a class=\"help-link\" href=\"{}\" target=\"_blank\">Where do I find my API key? &#8599;</a>",
            escape_html(help_url)
        )
    }
}

#[must_use]
pub fn build_authorize_html(page: &AuthorizePage) -> String {
    let server = escape_html(&page.server_name);
    let initial = escape_html(&page.server_initial);
    let access = access_section(page);
    let help = safe_help_link(&page.help_url);
    format!(
        r#"<!DOCTYPE html><html lang="en"><head><meta charset="UTF-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>Connect {server} &mdash; LiteLLM</title><style>.step{{display:none}}.step.show{{display:block}}.pill.sel{{font-weight:bold}}</style></head><body><div class="modal"><div id="s1" class="step show"><div class="dots"><div class="dot active"></div><div class="dot"></div></div><button class="close-btn" type="button" onclick="doCancel()" title="Close">&times;</button><div class="logos"><img src="/get_image?variant=monogram" class="logo-img" alt="LiteLLM"><span class="logo-arrow">&#8594;</span><div class="logo logo-s">{initial}</div></div><h2 class="step-title">Connect {server} MCP</h2><p class="step-subtitle">LiteLLM needs access to {server} to complete your request.</p><div class="info-box"><h4>How it works</h4><p>LiteLLM acts as a secure bridge. Your requests are routed through our MCP client directly to {server}&rsquo;s API.</p></div>{access}<button class="btn-primary" type="button" onclick="goStep2()">Continue to Authentication &rarr;</button><button class="btn-cancel" type="button" onclick="doCancel()">Cancel</button></div><div id="s2" class="step"><div class="step2-nav"><button class="back-btn" type="button" onclick="goStep1()">&#8592; Back</button><div class="dots"><div class="dot active"></div><div class="dot active"></div></div><button class="close-btn" style="position:static;" type="button" onclick="doCancel()" title="Close">&times;</button></div><div class="key-icon-wrap">&#128273;</div><h2 class="step-title" style="text-align:left;">Provide API Key</h2><p class="step-subtitle" style="text-align:left;">Enter your {server} API key to authorize this connection.</p><form method="POST" id="authForm" onsubmit="prepareSubmit()"><input type="hidden" name="client_id" value="{client_id}"><input type="hidden" name="redirect_uri" value="{redirect_uri}"><input type="hidden" name="code_challenge" value="{challenge}"><input type="hidden" name="code_challenge_method" value="{method}"><input type="hidden" name="state" value="{state}"><input type="hidden" name="server_id" value="{server_id}"><input type="hidden" name="duration" id="durInput" value="until_revoked"><label class="field-label">{server} API Key</label><input type="password" name="api_key" id="apiKey" class="key-input" placeholder="Enter your API key" required autofocus oninput="syncBtn()">{help}<div class="save-card"><div class="save-row"><span class="save-label">Save key for future use</span><label class="toggle"><input type="checkbox" id="saveToggle" onchange="toggleDur()"><span class="slider"></span></label></div><div id="durSection" class="duration-section" style="display:none;"><div class="duration-label">Duration</div><div class="pills"><div class="pill" onclick="selDur('1h',this)">1 hour</div><div class="pill sel" onclick="selDur('24h',this)">24 hours</div><div class="pill" onclick="selDur('7d',this)">7 days</div><div class="pill" onclick="selDur('30d',this)">30 days</div><div class="pill" onclick="selDur('until_revoked',this)">Until I revoke</div></div></div></div><div class="sec-note"><p>Your key is stored securely and transmitted over HTTPS. It is never shared with third parties.</p></div><button type="submit" class="btn-connect" id="connectBtn">Connect &amp; Authorize</button></form></div></div><script>function goStep2(){{document.getElementById('s1').classList.remove('show');document.getElementById('s2').classList.add('show')}}function goStep1(){{document.getElementById('s2').classList.remove('show');document.getElementById('s1').classList.add('show')}}function doCancel(){{if(window.opener)window.close();else window.history.back()}}function toggleDur(){{const on=document.getElementById('saveToggle').checked;document.getElementById('durSection').style.display=on?'block':'none'}}function selDur(val,el){{document.querySelectorAll('.pill').forEach(p=>p.classList.remove('sel'));el.classList.add('sel');document.getElementById('durInput').value=val}}function syncBtn(){{const btn=document.getElementById('connectBtn');if(document.getElementById('apiKey').value.length>0)btn.classList.add('ready');else btn.classList.remove('ready')}}function prepareSubmit(){{}}</script></body></html>"#,
        client_id = escape_html(&page.client_id),
        redirect_uri = escape_html(&page.redirect_uri),
        challenge = escape_html(&page.code_challenge),
        method = escape_html(&page.code_challenge_method),
        state = escape_html(&page.state),
        server_id = escape_html(&page.server_id)
    )
}

#[path = "byok_authorization_html/behavior/tests.rs"]
mod behavior;
#[path = "byok_authorization_html/escaping/tests.rs"]
mod escaping;
#[path = "byok_authorization_html/fields/tests.rs"]
mod fields;
#[path = "byok_authorization_html/help_url/tests.rs"]
mod help_url;
