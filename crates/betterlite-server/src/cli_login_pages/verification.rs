pub(super) const PAGE: &str = r#"
    <!doctype html>
    <html>
      <head>
        <title>betterlite CLI Login</title>
        <style>
          body {
            font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
            margin: 0;
            min-height: 100vh;
            display: flex;
            align-items: center;
            justify-content: center;
            background: #f8fafc;
            color: #0f172a;
          }
          main {
            width: min(420px, calc(100vw - 32px));
            background: #ffffff;
            border: 1px solid #e2e8f0;
            border-radius: 8px;
            padding: 28px;
            box-shadow: 0 12px 32px rgba(15, 23, 42, 0.08);
          }
          h1 { font-size: 22px; margin: 0 0 12px; }
          p { line-height: 1.5; margin: 0 0 18px; color: #334155; }
          label { display: block; font-weight: 600; margin-bottom: 8px; }
          input {
            box-sizing: border-box;
            width: 100%;
            padding: 12px;
            border: 1px solid #cbd5e1;
            border-radius: 6px;
            font-size: 20px;
            letter-spacing: 0.08em;
            text-transform: uppercase;
          }
          button {
            margin-top: 16px;
            width: 100%;
            padding: 12px;
            border: 0;
            border-radius: 6px;
            background: #0f172a;
            color: #ffffff;
            font-weight: 600;
            cursor: pointer;
          }
        </style>
      </head>
      <body>
        <main>
          <h1>Complete CLI Login</h1>
          <p>@@INSTRUCTIONS@@</p>
          <form method="post" action="@@VERIFY_URL@@">
            <input type="hidden" name="browser_complete_token" value="@@BROWSER_TOKEN@@" />
            <label for="user_code">Verification code</label>
            <input id="user_code" name="user_code" autocomplete="one-time-code"@@USER_CODE_VALUE@@ required autofocus />
            <button type="submit">Continue</button>
          </form>
        </main>
      </body>
    </html>
    "#;
