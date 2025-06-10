use std::{
    collections::HashMap,
    error::Error,
    fs,
};

use oauth2::{
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
use pyo3::prelude::*;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value as JSONValue;
use toml;
use url::Url;

use super::AudioQuality;

/// Struct used to persist session info.
#[derive(Deserialize, Serialize)]
struct TidalSessionInfo {
    access_token: String,
    refresh_token: String,
}

/// A currently logged in Tidal session.
#[derive(Debug)]
pub struct Session {
    access_token: String,
    pkce_access_token: String,
    country_code: String,
    request_client: Client,
    /// A reference to the tidalapi.Session Python object. Used for the unofficial Tidal API.
    pub(super) py_tidalapi_session: PyObject
}

impl Session {
    /// Base URL of the official Tidal API.
    const BASE_URL: &str = "https://openapi.tidal.com/v2";
    /// Base URL of the unofficial Tidal API.
    const UNOFFICIAL_BASE_URL: &str = "https://api.tidal.com/v1";

    /// Returns a new logged in `Session`.
    /// 
    /// If there is no existing previous session, the user must follow a link to login to Tidal.
    pub fn new(client_id: &str, client_secret: &str) -> Result<Self, String> {
        let request_client = Client::new();

        let session_exists = fs::exists("tidal-session.toml")
            .map_err(|e| format!("{e}"))?;
        
        let pkce_session: TidalSessionInfo = if session_exists {
            // Restore existing Tidal session.
            let toml_str = fs::read_to_string("tidal-session.toml")
                .map_err(|e| format!("{e}"))?;
            let existing_session: TidalSessionInfo = toml::from_str(&toml_str)
                .map_err(|e| format!("{e}"))?;

            // Get new access token from existing refresh token.
            let mut body = HashMap::new();
            body.insert("grant_type", "refresh_token");
            body.insert("refresh_token", &existing_session.refresh_token);
            body.insert("client_id", &client_id);
            let res = request_client.post("https://auth.tidal.com/v1/oauth2/token")
                .form(&body)
                .send()
                .map_err(|e| format!("Unable to get new access token with refresh token: {}", e.to_string()))?;
            
            let json: JSONValue = res.json()
                .map_err(|e| format!("Unable to parse API response into JSON: {}", e.to_string()))?;
            let new_access_token = json["access_token"].as_str()
                .ok_or("Failed to get access token")?
                .to_string();

            TidalSessionInfo {
                access_token: new_access_token,
                refresh_token: existing_session.refresh_token,
            }
        } else {
            // Create a new Tidal session by having the user login with their credentials.
            Self::new_ouath_pkce_login(client_id, client_secret)
                .map_err(|e| format!("{e}"))?
        };

        // Store Tidal session info to file.
        let toml_str = toml::to_string(&pkce_session)
            .map_err(|e| format!("{e}"))?;
        fs::write("tidal-session.toml", toml_str)
            .map_err(|e| format!("{e}"))?;

        // Get unofficial Tidal API session.
        let (py_tidalapi_session, access_token, country_code) = Self::new_python_tidalapi_session()?;

        Ok(Self {
            access_token,
            pkce_access_token: pkce_session.access_token,
            country_code,
            request_client,
            py_tidalapi_session,
        })
    }

    /// Performs the OAuth2 PKCE Tidal login sequence.
    fn new_ouath_pkce_login(client_id: &str, client_secret: &str) -> Result<TidalSessionInfo, Box<dyn Error>> {
        // Create an OAuth2 client.
        let client = BasicClient::new(ClientId::new(client_id.to_string()))
            .set_client_secret(ClientSecret::new(client_secret.to_string()))
            .set_auth_uri(AuthUrl::new("https://login.tidal.com/authorize".to_string())?)
            .set_token_uri(TokenUrl::new("https://auth.tidal.com/v1/oauth2/token".to_string())?)
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
            .set_pkce_challenge(pkce_challenge)
            .url();

        println!("Please open this URL in your web browser to login to Tidal:");
        println!("\n{}\n", auth_url);
        println!("After logging in, copy the entire URL from your browser's address bar, paste it here, and press ENTER.");
        println!("After pressing ENTER, you may have to log in a second time with another link to enable playback and other unofficial API features.");

        // Parse redirect URL.
        let mut redirect_url = String::new();
        std::io::stdin().read_line(&mut redirect_url)?;
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

        let token_result = 
            client
                .exchange_code(received_code)
                .set_pkce_verifier(pkce_verifier)
                .request(&http_client)?;

        let access_token = token_result.access_token().secret().to_string();
        let refresh_token = token_result.refresh_token().ok_or("No refresh token")?.secret().to_string();
        
        Ok(TidalSessionInfo {
            access_token,
            refresh_token,
        })
    }

    /// Prints a login link for the user, and returns a Python tidalapi session object as well as the access token and country code upon successful login.
    fn new_python_tidalapi_session() -> Result<(PyObject, String, String), String> {
        pyo3::prepare_freethreaded_python();

        let result = Python::with_gil(|py| -> PyResult<Option<(PyObject, String, String)>> {
            let tidalapi = PyModule::import(py, "tidalapi")?;
            let session = tidalapi.call_method0("Session")?;

            let pathlib = PyModule::import(py, "pathlib")?;
            let path_type = pathlib.getattr("Path")?;
            let oauth_file_path = path_type.call1(("unofficial-tidal-session.json",))?;

            let login_result = session.call_method1("login_session_file", (oauth_file_path,))?;
            let login_result: bool = login_result.extract()?;
            if login_result == false {
                return Ok(None);
            }

            let access_token: Option<String> = session.getattr("access_token")?.extract()?;
            let country_code: Option<String> = session.getattr("country_code")?.extract()?;
            if access_token.is_none() || country_code.is_none() {
                return Ok(None);
            }

            Ok(Some(
                (session.unbind(), access_token.unwrap(), country_code.unwrap())
            ))
        });

        match result {
            Err(err) => Err(format!("A Python exception occurred:\n{}", err.to_string())),
            Ok(None) => Err(String::from("Login failure")),
            Ok(Some(session_info)) => Ok(session_info),
        }
    }

    /// Makes a GET request to the Tidal API.
    /// 
    /// Returns the JSON from key "data" on a successful response.
    pub(super) fn get(&self, endpoint: &str) -> Result<JSONValue, String> {
        let url = format!("{}{}?countryCode={}", Self::BASE_URL, endpoint, self.country_code);
        let res = self.request_client.get(url)
            .bearer_auth(&self.access_token)
            .send()
            .map_err(|e| format!("Unable to send GET request to {}: {}", endpoint, e.to_string()))?;

        if !res.status().is_success() {
            return Err(format!("GET request to {} failed with status code {}", endpoint, res.status()));
        }

        let mut json: JSONValue = res.json()
            .map_err(|e| format!("Unable to parse API response into JSON: {}", e.to_string()))?;
        Ok(json["data"].take())
    }

    /// Makes a GET request to the Tidal API using the pkce session.
    /// 
    /// Returns the JSON from key "data" on a successful response.
    pub(super) fn get_with_pkce(&self, endpoint: &str) -> Result<JSONValue, String> {
        let url = format!("{}{}?countryCode={}", Self::BASE_URL, endpoint, self.country_code);
        let res = self.request_client.get(url)
            .bearer_auth(&self.pkce_access_token)
            .send()
            .map_err(|e| format!("Unable to send GET request to {}: {}", endpoint, e.to_string()))?;

        if !res.status().is_success() {
            return Err(format!("GET request to {} failed with status code {}", endpoint, res.status()));
        }

        let mut json: JSONValue = res.json()
            .map_err(|e| format!("Unable to parse API response into JSON: {}", e.to_string()))?;
        Ok(json["data"].take())
    }

    /// Makes a GET request to the unofficial Tidal API.
    /// 
    /// Returns the JSON on a successful response.
    pub(super) fn get_unofficial(&self, endpoint: &str) -> Result<JSONValue, String> {
        let url = if endpoint.contains("?") {
            format!("{}{}&countryCode={}", Self::UNOFFICIAL_BASE_URL, endpoint, self.country_code)
        } else {
            format!("{}{}?countryCode={}", Self::UNOFFICIAL_BASE_URL, endpoint, self.country_code)
        };

        let res = self.request_client.get(url)
            .bearer_auth(&self.access_token)
            .send()
            .map_err(|e| format!("Unable to send (unofficial) GET request to {}: {}", endpoint, e.to_string()))?;

        if !res.status().is_success() {
            return Err(format!("(unofficial) GET request to {} failed with status code {}", endpoint, res.status()));
        }

        let json: JSONValue = res.json()
            .map_err(|e| format!("Unable to parse (unofficial) API response into JSON: {}", e.to_string()))?;

        Ok(json)
    }

    /// Sets the audio quality setting used for playback.
    pub fn set_audio_quality(&self, quality: AudioQuality) -> Result<(), String> {
        let result = Python::with_gil(|py| -> PyResult<()> {
            let audio_quality_str = quality.to_tidalapi_string();
            self.py_tidalapi_session.setattr(py, "audio_quality", audio_quality_str)
        });

        match result {
            Err(err) => Err(format!("A Python exception occurred:\n{}", err.to_string())),
            _ => Ok(()),
        }
    }
}
