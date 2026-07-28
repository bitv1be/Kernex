# Authentication

Kernex uses one authentication-profile system in both the CLI and desktop application. A profile has a name, provider, method, and non-secret metadata. Secret values live in the operating system's native credential store and are resolved only for an approved provider request.

## API keys

Run the interactive picker:

```bash
kernex auth login
```

Or choose the provider and method explicitly:

```bash
kernex auth login --provider openai-compatible --method api-key --profile personal
kernex auth login --provider anthropic --method environment \
  --environment-variable ANTHROPIC_API_KEY --profile work
```

API-key input is hidden. Keyring-backed profiles store the value through Windows Credential Manager, macOS Keychain, or Linux Secret Service. Environment profiles store only the variable name. Use `kernex auth status`, `kernex auth use PROFILE`, and `kernex auth logout PROFILE` to inspect, select, or remove profiles without printing a credential.

Custom OpenAI-compatible header credentials can still be mapped from environment variables with `--header-env HEADER=ENVIRONMENT_VARIABLE`. Literal header secrets are not accepted in project configuration.

## Google OAuth with PKCE

Kernex includes Google's official authorization and token endpoints for Gemini. Create an OAuth public-client ID in a Google Cloud project that has the required API enabled, then run:

```bash
kernex auth login --provider gemini --method o-auth \
  --client-id YOUR_DESKTOP_CLIENT_ID \
  --google-project YOUR_CLOUD_PROJECT_ID \
  --profile google
```

`GOOGLE_OAUTH_CLIENT_ID` and `GOOGLE_CLOUD_PROJECT` can supply those values instead. Kernex requests Google's documented Cloud Platform and Generative Language scopes and sends the non-secret project ID as `x-goog-user-project` for quota. It opens the system browser, listens on a random loopback port, validates the returned state, exchanges the code with its PKCE verifier, and stores the resulting access/refresh tokens in the native keyring. Expired access tokens are refreshed automatically when a refresh token is available.

The OAuth client's redirect configuration must permit a loopback redirect for an installed application. Authorization availability, consent-screen requirements, scopes, and account policy remain controlled by Google.

## Other providers and custom OAuth

Kernex does not copy private sign-in flows from OpenAI, Anthropic, or other coding applications. Use API credentials when a provider does not publish a third-party installed-app OAuth flow.

The `custom` adapter accepts explicit authorization/token URLs and scopes for a provider that documents an official OAuth 2.0 public-client PKCE flow:

```bash
kernex auth login --provider custom --method o-auth --profile company \
  --client-id PUBLIC_CLIENT_ID \
  --authorization-url https://provider.example/oauth/authorize \
  --token-url https://provider.example/oauth/token \
  --scope models.read --scope inference.write
```

Verify those values against the provider's documentation. Kernex never scrapes a login page, intercepts a password, or imports another application's tokens.

## Desktop controls

Open Settings, then Authentication. The same named profiles can be created from an API key, environment variable, or supported OAuth flow, selected for the active provider, and removed. Profiles created in either interface are immediately available to the other.

## Troubleshooting

- `credential unavailable` means the profile metadata exists but its keyring item or referenced environment variable cannot currently be resolved.
- On Linux, ensure a Secret Service implementation is installed and unlocked in the current session.
- Browser callbacks bind only to loopback. Keep the CLI or desktop application running until the provider redirects back.
- A rejected or expired refresh token requires signing in again; logout removes both metadata and stored credential material.
