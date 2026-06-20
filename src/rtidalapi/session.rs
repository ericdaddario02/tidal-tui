use std::{
    fs,
    path::{
        Path,
        PathBuf,
    },
    sync::Mutex,
};

use base64::{
    engine::general_purpose::STANDARD as BASE64, 
    Engine as _
};
use chrono::Utc;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value as JSONValue;
use toml;

#[cfg(not(feature = "unofficial"))]
mod official_only_imports {
    pub use std::error::Error;
    pub use oauth2::{
        AuthorizationCode,
        AuthUrl,
        basic::BasicClient,
        ClientId,
        ClientSecret,
        CsrfToken,
        PkceCodeChallenge,
        RedirectUrl,
        Scope,
        TokenResponse,
        TokenUrl
    };
    pub use url::Url;
}

#[cfg(not(feature = "unofficial"))]
use official_only_imports::*;

use super::AudioQuality;

/// Struct used to persist session info.
#[derive(Debug, Deserialize, Serialize)]
struct SessionInfo {
    access_token: String,
    refresh_token: String,
    expires_at: i64,
}

/// A currently logged in Tidal session.
#[derive(Debug)]
pub struct Session {
    session_info: Mutex<SessionInfo>,
    client_id: String,
    client_secret: String,
    country_code: String,
    session_file: PathBuf,
    request_client: Client,
    audio_quality: Mutex<AudioQuality>,
}

impl Session {
    /// Base URL of the official Tidal API.
    const BASE_URL: &str = "https://openapi.tidal.com/v2";

    /// URL for the token endpoint.
    const TOKEN_URL: &str = "https://auth.tidal.com/v1/oauth2/token";

    /// Returns a new logged in `Session`.
    /// 
    /// If there is no existing previous session, the user must follow a link to login to Tidal. \
    /// `session_folder_path` is the directory path that the session info files will be stored.
    /// 
    /// If the `unofficial` feature is enabled, an unofficial session is created instead and `country_code` is ignored.
    #[allow(unused_variables)]
    pub fn new(client_id: &str, client_secret: &str, country_code: &str, session_folder_path: &str) -> Result<Self, String> {
        let request_client = Client::new();

        fs::create_dir_all(session_folder_path)
            .map_err(|e| format!("{e}"))?;
        
        let session_file = Path::new(session_folder_path).join("tidal-session.toml");

        #[cfg(not(feature = "unofficial"))]
        let (client_id, client_secret) = (client_id.to_owned(), client_secret.to_owned());

        #[cfg(feature = "unofficial")]
        let (client_id, client_secret) = Self::get_unofficial_client_id_and_secret();
            
        let session_info = Self::get_session(
            &request_client,
            &session_file,
            &client_id,
            &client_secret
        )?;

        #[cfg(not(feature = "unofficial"))]
        let country_code = country_code.to_string();

        #[cfg(feature = "unofficial")]
        let country_code = Self::fetch_country_code(&request_client, &session_info.access_token)?;

        Ok(Self {
            session_info: Mutex::new(session_info),
            client_id,
            client_secret,
            country_code,
            session_file,
            request_client,
            audio_quality: Mutex::new(AudioQuality::Max),
        })
    }

    /// Restores or creates a new session and returns the session info.
    /// 
    /// If using the `unofficial` feature, a device auth session is used.
    /// Otherwise, a PKCE OAuth2 session is used.
    fn get_session(request_client: &Client, session_file: &Path, client_id: &str, client_secret: &str) -> Result<SessionInfo, String> {
        // Try to restore from file if it exists.
        if session_file.exists() {
            let toml_str = fs::read_to_string(session_file)
                .map_err(|e| format!("{e}"))?;

            if let Ok(existing) = toml::from_str::<SessionInfo>(&toml_str) {
                // Get new access token from existing refresh token.
                match Self::refresh_access_token(request_client, &existing.refresh_token, client_id, client_secret) {
                    Ok(session_info) => {
                        let toml_str = toml::to_string(&session_info)
                            .map_err(|e| format!("{e}"))?;
                        fs::write(session_file, toml_str)
                            .map_err(|e| format!("{e}"))?;

                        return Ok(session_info);
                    },
                    Err(e) => {
                        eprintln!("Failed to refresh access token, performing new login: {}", e);
                    },
                }
            }
        }

        #[cfg(not(feature = "unofficial"))]
        // No valid session — perform new PKCE login.
        let new_session = Self::new_ouath_pkce_login(client_id, client_secret)
            .map_err(|e| format!("{e}"))?;

        #[cfg(feature = "unofficial")]
        // No valid session — perform new device auth login.
        let new_session = Self::new_device_auth_login(request_client, client_id, client_secret)?;

        let toml_str = toml::to_string(&new_session)
            .map_err(|e| format!("{e}"))?;
        fs::write(session_file, toml_str)
            .map_err(|e| format!("{e}"))?;

        Ok(new_session)
    }

    /// Checks if this `Session`'s current access token is expired,
    /// refreshes it if needed, and returns a valid access token.
    pub fn refresh_if_needed(&self) -> Result<String, String> {
        let mut session_info = self.session_info.lock().unwrap();

        if session_info.expires_at <= Utc::now().timestamp() {
            let new_session_info = Self::refresh_access_token(
                &self.request_client, 
                &session_info.refresh_token, 
                &self.client_id, 
                &self.client_secret
            )?;

            *session_info = new_session_info;

            let toml_str = toml::to_string(&(*session_info))
                .map_err(|e| format!("{e}"))?;
            fs::write(&self.session_file, toml_str)
                .map_err(|e| format!("{e}"))?;
        }

        Ok(session_info.access_token.clone())
    }

    /// Refreshes an access token using an existing refresh token.
    fn refresh_access_token(request_client: &Client, refresh_token: &str, client_id: &str, client_secret: &str) -> Result<SessionInfo, String> {
        let basic_auth = BASE64.encode(format!("{}:{}", client_id, client_secret));

        let res = request_client
            .post(Self::TOKEN_URL)
            .header("Authorization", format!("Basic {}", basic_auth))
            .form(&[
                ("client_id", client_id),
                ("refresh_token", refresh_token),
                ("grant_type", "refresh_token"),
            ])
            .send()
            .map_err(|e| format!("Token refresh request failed: {}", e))?;

        let json: JSONValue = res.json()
            .map_err(|e| format!("Failed to parse token refresh response: {}", e))?;

        if let Some(error) = json["error"].as_str() {
            return Err(format!("Token refresh error: {}", error));
        }

        let access_token = json["access_token"].as_str()
            .ok_or("No access_token in refresh response")?
            .to_string();
        let expires_in = json["expires_in"].as_i64()
                .ok_or("No expires_in in token reponse")?;
        let expires_at = Utc::now().timestamp() + expires_in;

        Ok(SessionInfo {
            access_token,
            refresh_token: refresh_token.to_string(),
            expires_at,
        })
    }

    /// Makes a GET request to the Tidal API.
    pub(super) fn get(&self, endpoint: &str) -> Result<JSONValue, String> {
        self.get_with_headers(endpoint, vec![])
    }

    /// Makes a GET request with headers to the Tidal API.
    pub(super) fn get_with_headers(&self, endpoint: &str, headers: Vec<(&str, &str)>) -> Result<JSONValue, String> {
        let url = if endpoint.contains("?") {
            format!("{}{}&countryCode={}", Self::BASE_URL, endpoint, self.country_code)
        } else {
            format!("{}{}?countryCode={}", Self::BASE_URL, endpoint, self.country_code)
        };

        let access_token = self.refresh_if_needed()?;

        let mut req = self.request_client.get(url)
            .bearer_auth(&access_token);

        for (key, val) in headers {
            req = req.header(key, val);
        }

        let res = req.send()
            .map_err(|e| format!("Unable to send GET request to {}: {}", endpoint, e.to_string()))?;

        if !res.status().is_success() {
            return Err(format!("GET request to {} failed with status code {}", endpoint, res.status()));
        }

        let json: JSONValue = res.json()
            .map_err(|e| format!("Unable to parse API response into JSON: {}", e.to_string()))?;
        Ok(json)
    }

    // TODO: remove mutex
    /// Sets the audio quality setting used for playback.
    pub fn set_audio_quality(&self, quality: AudioQuality) -> Result<(), String> {
        *self.audio_quality.lock().unwrap() = quality;

        Ok(())
    }

    /// Returns this sessions current audio quality setting.
    pub fn get_audio_quality(&self) -> AudioQuality {
        *self.audio_quality.lock().unwrap()
    }
}

#[cfg(not(feature = "unofficial"))]
impl Session {
    /// URL for the OAuth2 PKCE auth endpoint.
    const AUTH_URL: &str = "https://login.tidal.com/authorize";

    /// Performs the OAuth2 PKCE Tidal login sequence.
    fn new_ouath_pkce_login(client_id: &str, client_secret: &str) -> Result<SessionInfo, Box<dyn Error>> {
        // Create an OAuth2 client.
        let client = BasicClient::new(ClientId::new(client_id.to_string()))
            .set_client_secret(ClientSecret::new(client_secret.to_string()))
            .set_auth_uri(AuthUrl::new(Self::AUTH_URL.to_string())?)
            .set_token_uri(TokenUrl::new(Self::TOKEN_URL.to_string())?)
            .set_redirect_uri(RedirectUrl::new("http://localhost".to_string())?);

        let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();

        // Generate the full authorization URL.
        let (auth_url, csrf_token) = client
            .authorize_url(CsrfToken::new_random)
            .add_scope(Scope::new("user.read".to_string()))
            .add_scope(Scope::new("collection.read".to_string()))
            .add_scope(Scope::new("collection.write".to_string()))
            .add_scope(Scope::new("playlists.read".to_string()))
            .add_scope(Scope::new("playlists.write".to_string()))
            .add_scope(Scope::new("playback".to_string()))
            .set_pkce_challenge(pkce_challenge)
            .url();

        println!("Please open this URL in your web browser to login to Tidal:");
        println!("\n{}\n", auth_url);
        println!("After logging in, copy the entire URL from your browser's address bar, paste it here, and press ENTER.");

        // Parse redirect URL.
        let mut redirect_url = String::new();
        std::io::stdin().read_line(&mut redirect_url)?;
        println!("");

        let pasted_redirect_url = redirect_url.trim();
        let parsed_redirect_url = Url::parse(pasted_redirect_url)?;

        let (received_code, received_state) = {
            let query_pairs: std::collections::HashMap<_, _> = parsed_redirect_url
                .query_pairs()
                .collect();

            let code = AuthorizationCode::new(
                query_pairs.get("code")
                    .ok_or("No 'code' parameter found in redirect URL")?
                    .to_string(),
            );
            let state = CsrfToken::new(
                query_pairs.get("state")
                    .ok_or("No 'state' parameter found in redirect URL")?
                    .to_string(),
            );
            (code, state)
        };

        if received_state.secret() != csrf_token.secret() {
            return Err("CSRF token mismatch".into());
        }

        let http_client = reqwest::blocking::ClientBuilder::new()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("Client should build");

        let token_result = client
            .exchange_code(received_code)
            .set_pkce_verifier(pkce_verifier)
            .request(&http_client)?;

        let access_token = token_result.access_token().secret().to_string();
        let refresh_token = token_result.refresh_token().ok_or("No refresh_token")?.secret().to_string();
        let expires_in = token_result.expires_in().ok_or("No expires_in")?.as_secs() as i64;
        let expires_at = Utc::now().timestamp() + expires_in;
        
        Ok(SessionInfo {
            access_token,
            refresh_token,
            expires_at,
        })
    }
}

#[cfg(feature = "unofficial")]
impl Session {
    /// Base URL of the unofficial Tidal API.
    const UNOFFICIAL_BASE_URL: &str = "https://api.tidal.com/v1";

    /// URL for the unofficial Tidal API device auth.
    const DEVICE_AUTH_URL: &str   = "https://auth.tidal.com/v1/oauth2/device_authorization";

    /// Returns `(client_id, client_secret)` to be used for unofficial API auth.
    /// 
    /// The client_id and client_secret values were taken from https://github.com/EbbLabs/python-tidal/blob/main/tidalapi/session.py.
    fn get_unofficial_client_id_and_secret() -> (String, String) {
        let id_part1 = BASE64.decode(b"WmxneVNuaGtiVzUw").unwrap();
        let id_part2 = BASE64.decode(b"V2xkTE1HbDRWQT09").unwrap();
        
        let mut id_combined = id_part1;
        id_combined.extend_from_slice(&id_part2);
        
        let client_id = String::from_utf8(BASE64.decode(&id_combined).unwrap()).unwrap();

        let secret_part1 = BASE64.decode(b"TVU1dU9VRm1SRUZxZUhKblNrWktZa3RPVjB4bFFY").unwrap();
        let secret_part2 = BASE64.decode(b"bExSMVpIYlVsT2RWaFFVRXhJVmxoQmRuaEJaejA9").unwrap();
        
        let mut secret_combined = secret_part1;
        secret_combined.extend_from_slice(&secret_part2);
        
        let client_secret = String::from_utf8(BASE64.decode(&secret_combined).unwrap()).unwrap();

        (client_id, client_secret)
    }

    /// Performs the device authorization login flow using the unofficial Tidal client credentials.
    fn new_device_auth_login(request_client: &Client, client_id: &str, client_secret: &str) -> Result<SessionInfo, String> {
        let basic_auth = BASE64.encode(format!("{}:{}", client_id, client_secret));

        let res = request_client
            .post(Self::DEVICE_AUTH_URL)
            .header("Authorization", format!("Basic {}", basic_auth))
            .form(&[
                ("client_id", client_id),
                ("scope", "r_usr w_usr w_sub")
            ])
            .send()
            .map_err(|e| format!("Device auth request failed: {}", e))?;

        if !res.status().is_success() {
            let status = res.status();
            let body = res.text().unwrap_or_default();
            return Err(format!("Device auth request failed with {}: {}", status, body));
        }

        let json: JSONValue = res.json()
            .map_err(|e| format!("Failed to parse device auth response: {}", e))?;

        let device_code = json["deviceCode"].as_str()
            .ok_or("No deviceCode in device auth response")?
            .to_string();
        let user_code = json["userCode"].as_str()
            .ok_or("No userCode in device auth response")?
            .to_string();
        let verification_uri = json["verificationUriComplete"].as_str()
            .or_else(|| json["verificationUri"].as_str())
            .ok_or("No verificationUri in device auth response")?
            .to_string();
        let expires_in = json["expiresIn"].as_f64().unwrap_or(300.0);
        let interval = json["interval"].as_f64().unwrap_or(2.0);

        // Ask the user to log in.
        println!("Please open this URL in your web browser to login to Tidal:");
        println!("\n  https://{}\n", verification_uri);
        println!("Or visit https://tidal.com/activate and enter code: {}", user_code);

        // Poll until the user has logged in or the code expires.
        let poll_interval = std::time::Duration::from_secs_f64(interval.max(1.0));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs_f64(expires_in);

        loop {
            std::thread::sleep(poll_interval);

            if std::time::Instant::now() > deadline {
                return Err("Device authorization timed out — please try again.".to_string());
            }

            let poll_res = request_client
                .post(Self::TOKEN_URL)
                .header("Authorization", format!("Basic {}", basic_auth))
                .form(&[
                    ("client_id", client_id),
                    ("device_code", &device_code),
                    ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                    ("scope", "r_usr w_usr w_sub"),
                ])
                .send()
                .map_err(|e| format!("Token poll request failed: {}", e))?;

            let poll_json: JSONValue = poll_res.json()
                .map_err(|e| format!("Failed to parse token poll response: {}", e))?;

            if let Some(error) = poll_json["error"].as_str() {
                match error {
                    "authorization_pending" => continue,
                    "expired_token" => return Err("Device authorization expired — please try again.".to_string()),
                    other => return Err(format!("Authorization error: {}", other)),
                }
            }

            let access_token = poll_json["access_token"].as_str()
                .ok_or("No access_token in token response")?
                .to_string();
            let refresh_token = poll_json["refresh_token"].as_str()
                .ok_or("No refresh_token in token response")?
                .to_string();
            let expires_in = poll_json["expires_in"].as_i64()
                .ok_or("No expires_in in token reponse")?;
            let expires_at = Utc::now().timestamp() + expires_in; 

            return Ok(SessionInfo {
                access_token,
                refresh_token,
                expires_at,
            });
        }
    }

    /// Fetches the country code for the currently logged in user from the unofficial API.
    fn fetch_country_code(request_client: &Client, access_token: &str) -> Result<String, String> {
        let res = request_client
            .get("https://api.tidal.com/v1/sessions")
            .bearer_auth(access_token)
            .send()
            .map_err(|e| format!("Failed to fetch session info: {}", e))?;

        let json: JSONValue = res.json()
            .map_err(|e| format!("Failed to parse session info: {}", e))?;

        json["countryCode"].as_str()
            .ok_or_else(|| "No countryCode in session response".to_string())
            .map(|s| s.to_string())
    }

    /// Makes a GET request to the unofficial Tidal API.
    pub(super) fn get_unofficial(&self, endpoint: &str) -> Result<JSONValue, String> {
        let url = if endpoint.contains("?") {
            format!("{}{}&countryCode={}", Self::UNOFFICIAL_BASE_URL, endpoint, self.country_code)
        } else {
            format!("{}{}?countryCode={}", Self::UNOFFICIAL_BASE_URL, endpoint, self.country_code)
        };

        let access_token = self.refresh_if_needed()?;

        let res = self.request_client.get(url)
            .bearer_auth(&access_token)
            .send()
            .map_err(|e| format!("Unable to send (unofficial) GET request to {}: {}", endpoint, e.to_string()))?;

        if !res.status().is_success() {
            return Err(format!("(unofficial) GET request to {} failed with status code {}", endpoint, res.status()));
        }

        let json: JSONValue = res.json()
            .map_err(|e| format!("Unable to parse (unofficial) API response into JSON: {}", e.to_string()))?;

        Ok(json)
    }
}
