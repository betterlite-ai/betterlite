pub(super) const PAGE: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<meta name="referrer" content="no-referrer">
<title>Authorize CLI access - betterlite</title>
<style>
body {
    font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Oxygen, Ubuntu, Cantarell, sans-serif;
    background-color: #f8fafc;
    margin: 0;
    padding: 20px;
    display: flex;
    justify-content: center;
    align-items: center;
    min-height: 100vh;
    color: #1e293b;
}
.container {
    background-color: #fff;
    padding: 40px;
    border-radius: 8px;
    box-shadow: 0 2px 8px rgba(0, 0, 0, 0.1);
    width: 450px;
    max-width: 100%;
}
h1 { margin: 0 0 16px; font-size: 24px; font-weight: 600; }
p { margin: 0 0 12px; line-height: 1.5; }
code { background: #f1f5f9; padding: 2px 6px; border-radius: 4px; }
label { display: block; margin: 16px 0 6px; font-weight: 600; }
select { width: 100%; padding: 8px; border: 1px solid #cbd5e1; border-radius: 6px; font-size: 14px; }
.actions { display: flex; gap: 12px; margin-top: 24px; }
button { flex: 1; padding: 10px; border-radius: 6px; font-size: 15px; cursor: pointer; border: 1px solid #cbd5e1; }
.approve { background: #2563eb; color: #fff; border-color: #2563eb; }
.deny { background: #fff; color: #1e293b; }
</style>
</head>
<body>
<div class="container">
<h1>Authorize CLI access</h1>
<p>A command-line client at <code>@@ORIGIN@@</code> wants to call betterlite as <strong>@@USER@@</strong>.</p>
<p>Approving issues it a personal credential that expires within @@HOURS@@ hours. <code>lite logout</code> stops it from being renewed. Only approve if you started this sign-in yourself.</p>
<form method="post" action="@@COMPLETE_URL@@">
<input type="hidden" name="flow" value="@@FLOW_HANDLE@@">
@@TEAM_FIELD@@
<div class="actions">
<button type="submit" name="decision" value="deny" class="deny">Deny</button>
<button type="submit" name="decision" value="approve" class="approve">Approve</button>
</div>
</form>
</div>
</body>
</html>
"#;

use super::escape;

pub(super) fn team_field(teams: &[(String, String)]) -> String {
    match teams {
        [] => String::new(),
        [(team_id, team_label)] => format!(
            "<input type=\"hidden\" name=\"team_id\" value=\"{}\"><p>Requests are attributed to team <strong>{}</strong>.</p>",
            escape::text(team_id),
            escape::text(team_label)
        ),
        many => {
            let mut field = String::from(
                "<label for=\"team_id\">Attribute requests to team</label><select id=\"team_id\" name=\"team_id\">",
            );
            for (team_id, team_label) in many {
                field.push_str("<option value=\"");
                field.push_str(&escape::text(team_id));
                field.push_str("\">");
                field.push_str(&escape::text(team_label));
                field.push_str("</option>");
            }
            field.push_str("</select>");
            field
        }
    }
}
