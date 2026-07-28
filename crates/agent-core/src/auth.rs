use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::Utc;
use directories::ProjectDirs;
use keyring::v1::Entry;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::time::timeout;
use url::Url;
use zeroize::Zeroize;

use crate::provider::ProviderKind;

const KEYRING_SERVICE: &str = "dev.kernex.auth";
const OAUTH_CALLBACK_TIMEOUT_SECONDS: u64 = 300;

/// Sensitive authentication material. It cannot be serialized and is redacted in debug output.
pub struct SecretValue(String);

impl SecretValue {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretValue([REDACTED])")
    }
}

impl Drop for SecretValue {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthMethod {
    ApiKey,
    Environment,
    OAuthPkce,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthProfile {
    pub name: String,
    pub provider: ProviderKind,
    pub method: AuthMethod,
    pub environment_variable: Option<String>,
    pub account_label: Option<String>,
    pub expires_at: Option<i64>,
    pub oauth_client_id: Option<String>,
    pub oauth_token_url: Option<String>,
    #[serde(default)]
    pub oauth_scopes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth_resource_project: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuthStatus {
    pub profile: AuthProfile,
    pub active: bool,
    pub credential_available: bool,
    pub expired: bool,
}

#[derive(Debug, Clone)]
pub struct OAuthConfig {
    pub authorization_url: String,
    pub token_url: String,
    pub client_id: String,
    pub scopes: Vec<String>,
    pub extra_authorization_parameters: BTreeMap<String, String>,
    /// Non-secret quota/resource project required by some provider APIs.
    pub resource_project: Option<String>,
}

impl OAuthConfig {
    /// Official Google OAuth endpoints suitable for Gemini API access.
    pub fn google(client_id: impl Into<String>, resource_project: impl Into<String>) -> Self {
        Self {
            authorization_url: "https://accounts.google.com/o/oauth2/v2/auth".into(),
            token_url: "https://oauth2.googleapis.com/token".into(),
            client_id: client_id.into(),
            scopes: vec![
                "https://www.googleapis.com/auth/cloud-platform".into(),
                "https://www.googleapis.com/auth/generative-language.retriever".into(),
            ],
            extra_authorization_parameters: BTreeMap::from([
                ("access_type".into(), "offline".into()),
                ("prompt".into(), "consent".into()),
            ]),
            resource_project: Some(resource_project.into()),
        }
    }
}

pub trait CredentialVault: Send + Sync {
    fn set(&self, profile: &str, kind: &str, secret: &SecretValue) -> Result<(), AuthError>;
    fn get(&self, profile: &str, kind: &str) -> Result<Option<SecretValue>, AuthError>;
    fn delete(&self, profile: &str, kind: &str) -> Result<(), AuthError>;
}

#[derive(Debug, Default)]
pub struct KeyringVault;

impl KeyringVault {
    fn entry(profile: &str, kind: &str) -> Result<Entry, AuthError> {
        let username = format!("{profile}:{kind}");
        Entry::new(KEYRING_SERVICE, &username).map_err(|error| AuthError::Vault(error.to_string()))
    }
}

impl CredentialVault for KeyringVault {
    fn set(&self, profile: &str, kind: &str, secret: &SecretValue) -> Result<(), AuthError> {
        Self::entry(profile, kind)?
            .set_password(secret.expose())
            .map_err(|error| AuthError::Vault(error.to_string()))
    }

    fn get(&self, profile: &str, kind: &str) -> Result<Option<SecretValue>, AuthError> {
        match Self::entry(profile, kind)?.get_password() {
            Ok(value) => Ok(Some(SecretValue::new(value))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(AuthError::Vault(error.to_string())),
        }
    }

    fn delete(&self, profile: &str, kind: &str) -> Result<(), AuthError> {
        match Self::entry(profile, kind)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(AuthError::Vault(error.to_string())),
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct AuthCatalog {
    profiles: Vec<AuthProfile>,
    active_profiles: BTreeMap<String, String>,
}

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("Kernex could not determine a local configuration directory")]
    MissingConfigDirectory,
    #[error("could not access authentication metadata {path}: {source}")]
    MetadataIo {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("authentication metadata is invalid: {0}")]
    Metadata(#[from] toml::de::Error),
    #[error("authentication metadata could not be encoded: {0}")]
    MetadataEncode(#[from] toml::ser::Error),
    #[error("secure credential storage is unavailable: {0}")]
    Vault(String),
    #[error("authentication profile `{0}` does not exist")]
    MissingProfile(String),
    #[error("authentication profile `{0}` has no credential")]
    MissingCredential(String),
    #[error("environment variable `{0}` is not set")]
    MissingEnvironmentVariable(String),
    #[error("OAuth configuration is invalid: {0}")]
    InvalidOAuthConfiguration(String),
    #[error("could not open the system browser: {0}")]
    Browser(String),
    #[error("OAuth callback timed out")]
    CallbackTimedOut,
    #[error("OAuth callback was invalid: {0}")]
    InvalidCallback(String),
    #[error("OAuth authorization was rejected: {0}")]
    AuthorizationRejected(String),
    #[error("OAuth token request failed: {0}")]
    TokenRequest(String),
    #[error("OAuth credential was revoked or is no longer valid; sign in again")]
    RevokedCredential,
    #[error("authentication state is unavailable")]
    Unavailable,
}

pub struct AuthManager {
    path: PathBuf,
    catalog: Mutex<AuthCatalog>,
    vault: Arc<dyn CredentialVault>,
    client: Client,
}

impl AuthManager {
    pub fn open_default() -> Result<Self, AuthError> {
        let directories = ProjectDirs::from("dev", "Kernex", "Kernex")
            .ok_or(AuthError::MissingConfigDirectory)?;
        Self::open(
            directories.config_dir().join("auth.toml"),
            Arc::new(KeyringVault),
        )
    }

    pub fn open(
        path: impl AsRef<Path>,
        vault: Arc<dyn CredentialVault>,
    ) -> Result<Self, AuthError> {
        let path = path.as_ref().to_path_buf();
        let catalog = if path.exists() {
            let contents = fs::read_to_string(&path).map_err(|source| AuthError::MetadataIo {
                path: path.clone(),
                source,
            })?;
            toml::from_str(&contents)?
        } else {
            AuthCatalog::default()
        };
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(30))
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(|error| AuthError::TokenRequest(error.to_string()))?;
        Ok(Self {
            path,
            catalog: Mutex::new(catalog),
            vault,
            client,
        })
    }

    pub fn profiles(&self) -> Result<Vec<AuthProfile>, AuthError> {
        Ok(self
            .catalog
            .lock()
            .map_err(|_| AuthError::Unavailable)?
            .profiles
            .clone())
    }

    pub fn active_profile(&self, provider: ProviderKind) -> Result<Option<AuthProfile>, AuthError> {
        let catalog = self.catalog.lock().map_err(|_| AuthError::Unavailable)?;
        let Some(name) = catalog.active_profiles.get(&provider.to_string()) else {
            return Ok(None);
        };
        Ok(catalog
            .profiles
            .iter()
            .find(|profile| &profile.name == name)
            .cloned())
    }

    pub fn statuses(&self) -> Result<Vec<AuthStatus>, AuthError> {
        let catalog = self.catalog.lock().map_err(|_| AuthError::Unavailable)?;
        let mut statuses = Vec::with_capacity(catalog.profiles.len());
        for profile in &catalog.profiles {
            let active = catalog
                .active_profiles
                .get(&profile.provider.to_string())
                .is_some_and(|name| name == &profile.name);
            let credential_available = match profile.method {
                AuthMethod::Environment => profile
                    .environment_variable
                    .as_ref()
                    .is_some_and(|name| std::env::var_os(name).is_some()),
                AuthMethod::ApiKey => self.vault.get(&profile.name, "api_key")?.is_some(),
                AuthMethod::OAuthPkce => {
                    self.vault.get(&profile.name, "access_token")?.is_some()
                        || self.vault.get(&profile.name, "refresh_token")?.is_some()
                }
            };
            statuses.push(AuthStatus {
                profile: profile.clone(),
                active,
                credential_available,
                expired: profile
                    .expires_at
                    .is_some_and(|value| value <= Utc::now().timestamp()),
            });
        }
        Ok(statuses)
    }

    pub fn login_api_key(
        &self,
        name: impl Into<String>,
        provider: ProviderKind,
        secret: SecretValue,
    ) -> Result<AuthProfile, AuthError> {
        let name = validated_profile_name(name.into())?;
        self.vault.set(&name, "api_key", &secret)?;
        let profile = AuthProfile {
            name,
            provider,
            method: AuthMethod::ApiKey,
            environment_variable: None,
            account_label: None,
            expires_at: None,
            oauth_client_id: None,
            oauth_token_url: None,
            oauth_scopes: Vec::new(),
            oauth_resource_project: None,
        };
        self.upsert_and_activate(profile.clone())?;
        Ok(profile)
    }

    pub fn login_environment(
        &self,
        name: impl Into<String>,
        provider: ProviderKind,
        variable: impl Into<String>,
    ) -> Result<AuthProfile, AuthError> {
        let name = validated_profile_name(name.into())?;
        let variable = variable.into().trim().to_owned();
        if variable.is_empty() {
            return Err(AuthError::MissingEnvironmentVariable(variable));
        }
        let profile = AuthProfile {
            name,
            provider,
            method: AuthMethod::Environment,
            environment_variable: Some(variable),
            account_label: None,
            expires_at: None,
            oauth_client_id: None,
            oauth_token_url: None,
            oauth_scopes: Vec::new(),
            oauth_resource_project: None,
        };
        self.upsert_and_activate(profile.clone())?;
        Ok(profile)
    }

    pub fn set_active(&self, provider: ProviderKind, name: &str) -> Result<(), AuthError> {
        let mut catalog = self.catalog.lock().map_err(|_| AuthError::Unavailable)?;
        if !catalog
            .profiles
            .iter()
            .any(|profile| profile.provider == provider && profile.name == name)
        {
            return Err(AuthError::MissingProfile(name.to_owned()));
        }
        catalog
            .active_profiles
            .insert(provider.to_string(), name.to_owned());
        self.save_catalog(&catalog)
    }

    pub fn logout(&self, name: &str) -> Result<(), AuthError> {
        let mut catalog = self.catalog.lock().map_err(|_| AuthError::Unavailable)?;
        if !catalog.profiles.iter().any(|profile| profile.name == name) {
            return Err(AuthError::MissingProfile(name.to_owned()));
        }
        for kind in ["api_key", "access_token", "refresh_token"] {
            self.vault.delete(name, kind)?;
        }
        catalog.profiles.retain(|profile| profile.name != name);
        catalog
            .active_profiles
            .retain(|_, profile_name| profile_name != name);
        self.save_catalog(&catalog)
    }

    pub async fn resolve(&self, name: &str) -> Result<SecretValue, AuthError> {
        let profile = self
            .profiles()?
            .into_iter()
            .find(|profile| profile.name == name)
            .ok_or_else(|| AuthError::MissingProfile(name.to_owned()))?;
        match profile.method {
            AuthMethod::ApiKey => self
                .vault
                .get(name, "api_key")?
                .ok_or_else(|| AuthError::MissingCredential(name.to_owned())),
            AuthMethod::Environment => {
                let variable = profile
                    .environment_variable
                    .ok_or_else(|| AuthError::MissingEnvironmentVariable(String::new()))?;
                std::env::var(&variable)
                    .map(SecretValue::new)
                    .map_err(|_| AuthError::MissingEnvironmentVariable(variable))
            }
            AuthMethod::OAuthPkce => {
                let should_refresh = profile
                    .expires_at
                    .is_some_and(|expires| expires <= Utc::now().timestamp() + 60);
                if !should_refresh
                    && let Some(access_token) = self.vault.get(name, "access_token")?
                {
                    return Ok(access_token);
                }
                self.refresh_oauth(&profile).await
            }
        }
    }

    pub async fn login_oauth(
        &self,
        name: impl Into<String>,
        provider: ProviderKind,
        config: OAuthConfig,
    ) -> Result<AuthProfile, AuthError> {
        let name = validated_profile_name(name.into())?;
        validate_oauth_config(&config)?;
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|error| AuthError::InvalidCallback(error.to_string()))?;
        let callback_address = listener
            .local_addr()
            .map_err(|error| AuthError::InvalidCallback(error.to_string()))?;
        let redirect_uri = format!("http://127.0.0.1:{}/callback", callback_address.port());
        let verifier = random_url_token(64)?;
        let state = random_url_token(32)?;
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));

        let mut authorization = Url::parse(&config.authorization_url)
            .map_err(|error| AuthError::InvalidOAuthConfiguration(error.to_string()))?;
        {
            let mut query = authorization.query_pairs_mut();
            query
                .append_pair("response_type", "code")
                .append_pair("client_id", &config.client_id)
                .append_pair("redirect_uri", &redirect_uri)
                .append_pair("scope", &config.scopes.join(" "))
                .append_pair("state", &state)
                .append_pair("code_challenge", &challenge)
                .append_pair("code_challenge_method", "S256");
            for (key, value) in &config.extra_authorization_parameters {
                query.append_pair(key, value);
            }
        }
        webbrowser::open(authorization.as_str())
            .map_err(|error| AuthError::Browser(error.to_string()))?;

        let callback = timeout(
            Duration::from_secs(OAUTH_CALLBACK_TIMEOUT_SECONDS),
            receive_callback(listener, &state),
        )
        .await
        .map_err(|_| AuthError::CallbackTimedOut)??;
        let token = self
            .exchange_code(&config, &callback, &redirect_uri, &verifier)
            .await?;
        self.vault
            .set(&name, "access_token", &SecretValue::new(token.access_token))?;
        if let Some(refresh_token) = token.refresh_token {
            self.vault
                .set(&name, "refresh_token", &SecretValue::new(refresh_token))?;
        }
        let profile = AuthProfile {
            name,
            provider,
            method: AuthMethod::OAuthPkce,
            environment_variable: None,
            account_label: None,
            expires_at: token
                .expires_in
                .map(|seconds| Utc::now().timestamp().saturating_add(seconds as i64)),
            oauth_client_id: Some(config.client_id),
            oauth_token_url: Some(config.token_url),
            oauth_scopes: config.scopes,
            oauth_resource_project: config.resource_project,
        };
        self.upsert_and_activate(profile.clone())?;
        Ok(profile)
    }

    fn upsert_and_activate(&self, profile: AuthProfile) -> Result<(), AuthError> {
        let mut catalog = self.catalog.lock().map_err(|_| AuthError::Unavailable)?;
        catalog
            .profiles
            .retain(|existing| existing.name != profile.name);
        catalog.profiles.push(profile.clone());
        catalog
            .active_profiles
            .insert(profile.provider.to_string(), profile.name.clone());
        catalog
            .profiles
            .sort_by(|left, right| left.name.cmp(&right.name));
        self.save_catalog(&catalog)
    }

    fn save_catalog(&self, catalog: &AuthCatalog) -> Result<(), AuthError> {
        let Some(parent) = self.path.parent() else {
            return Err(AuthError::MissingConfigDirectory);
        };
        fs::create_dir_all(parent).map_err(|source| AuthError::MetadataIo {
            path: parent.to_path_buf(),
            source,
        })?;
        let encoded = toml::to_string_pretty(catalog)?;
        fs::write(&self.path, encoded).map_err(|source| AuthError::MetadataIo {
            path: self.path.clone(),
            source,
        })
    }

    async fn exchange_code(
        &self,
        config: &OAuthConfig,
        code: &str,
        redirect_uri: &str,
        verifier: &str,
    ) -> Result<OAuthTokenResponse, AuthError> {
        let response = self
            .client
            .post(&config.token_url)
            .form(&[
                ("grant_type", "authorization_code"),
                ("client_id", config.client_id.as_str()),
                ("code", code),
                ("redirect_uri", redirect_uri),
                ("code_verifier", verifier),
            ])
            .send()
            .await
            .map_err(|error| AuthError::TokenRequest(error.to_string()))?;
        parse_token_response(response).await
    }

    async fn refresh_oauth(&self, profile: &AuthProfile) -> Result<SecretValue, AuthError> {
        let refresh_token = self
            .vault
            .get(&profile.name, "refresh_token")?
            .ok_or_else(|| AuthError::MissingCredential(profile.name.clone()))?;
        let token_url = profile
            .oauth_token_url
            .as_deref()
            .ok_or_else(|| AuthError::InvalidOAuthConfiguration("missing token URL".into()))?;
        let client_id = profile
            .oauth_client_id
            .as_deref()
            .ok_or_else(|| AuthError::InvalidOAuthConfiguration("missing client ID".into()))?;
        let response = self
            .client
            .post(token_url)
            .form(&[
                ("grant_type", "refresh_token"),
                ("client_id", client_id),
                ("refresh_token", refresh_token.expose()),
            ])
            .send()
            .await
            .map_err(|error| AuthError::TokenRequest(error.to_string()))?;
        let token = match parse_token_response(response).await {
            Ok(token) => token,
            Err(error) => {
                self.vault.delete(&profile.name, "access_token")?;
                if matches!(error, AuthError::RevokedCredential) {
                    self.vault.delete(&profile.name, "refresh_token")?;
                }
                return Err(error);
            }
        };
        let access_token = SecretValue::new(token.access_token);
        self.vault
            .set(&profile.name, "access_token", &access_token)?;
        if let Some(refresh) = token.refresh_token {
            self.vault
                .set(&profile.name, "refresh_token", &SecretValue::new(refresh))?;
        }
        if let Some(expires_in) = token.expires_in {
            let mut catalog = self.catalog.lock().map_err(|_| AuthError::Unavailable)?;
            if let Some(saved) = catalog
                .profiles
                .iter_mut()
                .find(|saved| saved.name == profile.name)
            {
                saved.expires_at = Some(Utc::now().timestamp().saturating_add(expires_in as i64));
            }
            self.save_catalog(&catalog)?;
        }
        Ok(access_token)
    }
}

#[derive(Debug, Deserialize)]
struct OAuthTokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
}

async fn parse_token_response(
    response: reqwest::Response,
) -> Result<OAuthTokenResponse, AuthError> {
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| AuthError::TokenRequest(error.to_string()))?;
    if !status.is_success() {
        let parsed = serde_json::from_str::<serde_json::Value>(&body).ok();
        if parsed
            .as_ref()
            .and_then(|value| value.get("error"))
            .and_then(|value| value.as_str())
            == Some("invalid_grant")
        {
            return Err(AuthError::RevokedCredential);
        }
        let message = parsed
            .and_then(|value| {
                value
                    .get("error_description")
                    .or_else(|| value.get("error"))
                    .and_then(|value| value.as_str())
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| format!("provider returned HTTP {}", status.as_u16()));
        return Err(AuthError::TokenRequest(message));
    }
    serde_json::from_str(&body).map_err(|error| AuthError::TokenRequest(error.to_string()))
}

async fn receive_callback(
    listener: TcpListener,
    expected_state: &str,
) -> Result<String, AuthError> {
    let (mut stream, _) = listener
        .accept()
        .await
        .map_err(|error| AuthError::InvalidCallback(error.to_string()))?;
    let mut buffer = vec![0u8; 16 * 1024];
    let length = stream
        .read(&mut buffer)
        .await
        .map_err(|error| AuthError::InvalidCallback(error.to_string()))?;
    let request = String::from_utf8_lossy(&buffer[..length]);
    let target = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .ok_or_else(|| AuthError::InvalidCallback("missing request target".into()))?;
    let callback = Url::parse(&format!("http://127.0.0.1{target}"))
        .map_err(|error| AuthError::InvalidCallback(error.to_string()))?;
    let values: BTreeMap<_, _> = callback.query_pairs().into_owned().collect();
    let received_state = values
        .get("state")
        .ok_or_else(|| AuthError::InvalidCallback("missing state".into()))?;
    if expected_state
        .as_bytes()
        .ct_eq(received_state.as_bytes())
        .unwrap_u8()
        != 1
    {
        send_callback_response(&mut stream, false).await?;
        return Err(AuthError::InvalidCallback("state did not match".into()));
    }
    if let Some(error) = values.get("error") {
        send_callback_response(&mut stream, false).await?;
        return Err(AuthError::AuthorizationRejected(error.clone()));
    }
    let code = values
        .get("code")
        .cloned()
        .ok_or_else(|| AuthError::InvalidCallback("missing authorization code".into()))?;
    send_callback_response(&mut stream, true).await?;
    Ok(code)
}

async fn send_callback_response(
    stream: &mut tokio::net::TcpStream,
    success: bool,
) -> Result<(), AuthError> {
    let message = if success {
        "Authentication complete. You can return to Kernex."
    } else {
        "Authentication was not completed. Return to Kernex for details."
    };
    let body = format!(
        "<!doctype html><meta charset=\"utf-8\"><title>Kernex authentication</title><style>body{{font:16px system-ui;margin:4rem;max-width:44rem}}</style><h1>Kernex</h1><p>{message}</p>"
    );
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(response.as_bytes())
        .await
        .map_err(|error| AuthError::InvalidCallback(error.to_string()))
}

fn validate_oauth_config(config: &OAuthConfig) -> Result<(), AuthError> {
    if config.client_id.trim().is_empty() {
        return Err(AuthError::InvalidOAuthConfiguration(
            "client ID cannot be empty".into(),
        ));
    }
    if config
        .resource_project
        .as_ref()
        .is_some_and(|project| project.trim().is_empty())
    {
        return Err(AuthError::InvalidOAuthConfiguration(
            "resource project cannot be empty".into(),
        ));
    }
    for (label, value) in [
        ("authorization URL", &config.authorization_url),
        ("token URL", &config.token_url),
    ] {
        let url = Url::parse(value)
            .map_err(|error| AuthError::InvalidOAuthConfiguration(error.to_string()))?;
        if url.scheme() != "https" && !url.host_str().is_some_and(is_loopback_host) {
            return Err(AuthError::InvalidOAuthConfiguration(format!(
                "{label} must use HTTPS unless it targets localhost"
            )));
        }
    }
    Ok(())
}

fn is_loopback_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

fn validated_profile_name(name: String) -> Result<String, AuthError> {
    let trimmed = name.trim();
    if trimmed.is_empty()
        || trimmed.len() > 80
        || !trimmed
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "-_.".contains(character))
    {
        return Err(AuthError::InvalidOAuthConfiguration(
            "profile names may contain only letters, digits, dash, underscore, and dot".into(),
        ));
    }
    Ok(trimmed.to_owned())
}

fn random_url_token(length: usize) -> Result<String, AuthError> {
    let mut bytes = vec![0u8; length];
    getrandom::fill(&mut bytes)
        .map_err(|error| AuthError::InvalidOAuthConfiguration(error.to_string()))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct MemoryVault(Mutex<BTreeMap<(String, String), String>>);

    impl CredentialVault for MemoryVault {
        fn set(&self, profile: &str, kind: &str, secret: &SecretValue) -> Result<(), AuthError> {
            self.0
                .lock()
                .unwrap()
                .insert((profile.into(), kind.into()), secret.expose().into());
            Ok(())
        }

        fn get(&self, profile: &str, kind: &str) -> Result<Option<SecretValue>, AuthError> {
            Ok(self
                .0
                .lock()
                .unwrap()
                .get(&(profile.into(), kind.into()))
                .cloned()
                .map(SecretValue::new))
        }

        fn delete(&self, profile: &str, kind: &str) -> Result<(), AuthError> {
            self.0
                .lock()
                .unwrap()
                .remove(&(profile.into(), kind.into()));
            Ok(())
        }
    }

    fn temporary_catalog() -> PathBuf {
        std::env::temp_dir().join(format!(
            "kernex-auth-test-{}-{}.toml",
            std::process::id(),
            random_url_token(8).unwrap()
        ))
    }

    #[tokio::test]
    async fn api_keys_live_in_the_vault_not_metadata() {
        let path = temporary_catalog();
        let manager = AuthManager::open(&path, Arc::new(MemoryVault::default())).unwrap();
        manager
            .login_api_key(
                "personal",
                ProviderKind::OpenAiCompatible,
                SecretValue::new("super-secret"),
            )
            .unwrap();

        let metadata = fs::read_to_string(&path).unwrap();
        assert!(!metadata.contains("super-secret"));
        assert_eq!(
            manager.resolve("personal").await.unwrap().expose(),
            "super-secret"
        );

        fs::remove_file(path).unwrap();
    }

    #[test]
    fn oauth_requires_https_except_for_loopback_testing() {
        let invalid = OAuthConfig {
            authorization_url: "http://example.com/auth".into(),
            token_url: "https://example.com/token".into(),
            client_id: "client".into(),
            scopes: Vec::new(),
            extra_authorization_parameters: BTreeMap::new(),
            resource_project: None,
        };
        assert!(validate_oauth_config(&invalid).is_err());

        let local = OAuthConfig {
            authorization_url: "http://127.0.0.1/auth".into(),
            token_url: "http://localhost/token".into(),
            ..invalid
        };
        assert!(validate_oauth_config(&local).is_ok());
    }

    #[test]
    fn google_oauth_uses_documented_scopes_and_resource_project() {
        let config = OAuthConfig::google("desktop-client", "cloud-project");
        assert!(
            config
                .scopes
                .iter()
                .any(|scope| scope == "https://www.googleapis.com/auth/cloud-platform")
        );
        assert!(config.scopes.iter().any(|scope| {
            scope == "https://www.googleapis.com/auth/generative-language.retriever"
        }));
        assert_eq!(config.resource_project.as_deref(), Some("cloud-project"));
    }
}
