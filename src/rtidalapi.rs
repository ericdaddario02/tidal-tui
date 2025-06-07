use std::{
    collections::HashMap,
    error::Error,
    fs,
    sync::Arc,
    time::Duration
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
use once_cell::sync::OnceCell;
use pyo3::prelude::*;
use regex::Regex;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value as JSONValue;
use toml;
use url::Url;

/// Audio quality options in Tidal.
pub enum AudioQuality {
    /// 96 kbps
    Low96,
    /// 320 kbps
    Low320,
    /// 16-bit, 44.1 kHz
    High,
    // Max quality not currently supported.
    // /// Up to 24-bit, 192 kHz
    // Max,
} 

impl AudioQuality {
    /// Returns the string used by the Python tidalapi corresponding to this audio quality setting.
    fn to_tidalapi_string(&self) -> String {
        match self {
            Self::Low96 => String::from("LOW"),
            Self::Low320 => String::from("HIGH"),
            Self::High => String::from("LOSSLESS"),
            // Max quality not currently supported.
            // Self::Max => String::from("HI_RES_LOSSLES"),
        }
    }
}

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
    user_id: String,
    request_client: Client,
    /// A reference to the tidalapi.Session Python object. Used for the unofficial Tidal API.
    py_tidalapi_session: PyObject
}

impl Session {
    /// Base URL of the official Tidal API.
    const BASE_URL: &str = "https://openapi.tidal.com/v2";

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

        // Get user_id and country_code.
        let users_me_endpoint = "/users/me";
        let url = format!("{}{}", Self::BASE_URL, users_me_endpoint);
        let res = request_client.get(url)
            .bearer_auth(&pkce_session.access_token)
            .send()
            .map_err(|e| format!("Unable to send GET request to {}: {}", users_me_endpoint, e.to_string()))?;

        let mut json: JSONValue = res.json()
            .map_err(|e| format!("Unable to parse API response into JSON: {}", e.to_string()))?;
        let mut data_json = json["data"].take();

        let user_id = data_json["id"].as_str()
            .ok_or("Failed to get user id")?
            .to_string();
        let country_code = data_json["attributes"].take()["country"].as_str()
            .ok_or("Failed to get country code")?
            .to_string();

        // Get unofficial Tidal API session.
        let (py_tidalapi_session, access_token) = Self::new_python_tidalapi_session()?;

        Ok(Self {
            access_token,
            pkce_access_token: pkce_session.access_token,
            country_code,
            user_id,
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

    /// Prints a login link for the user, and returns a Python tidalapi session object as well as the access token upon successful login.
    fn new_python_tidalapi_session() -> Result<(PyObject, String), String> {
        pyo3::prepare_freethreaded_python();

        let result = Python::with_gil(|py| -> PyResult<Option<(PyObject, String)>> {
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
            if access_token.is_none() {
                return Ok(None);
            }

            Ok(Some(
                (session.unbind(), access_token.unwrap())
            ))
        });

        match result {
            Err(err) => Err(format!("A Python exception occurred:\n{}", err.to_string())),
            Ok(None) => Err(String::from("Login failure")),
            Ok(Some(session)) => Ok(session),
        }
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

    /// Makes a GET request to the Tidal API.
    /// 
    /// Returns the JSON from key "data" in a successful response.
    fn get(&self, endpoint: &str) -> Result<JSONValue, String> {
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
    /// Returns the JSON from key "data" in a successful response.
    fn get_with_pkce(&self, endpoint: &str) -> Result<JSONValue, String> {
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
}

/// A Tidal track.
#[derive(Debug)]
pub struct Track {
    session: Arc<Session>,
    pub id: String,
    pub attributes: TrackAttributes,

    // The following fields are used to cache relationship API results.
    album: OnceCell<Album>,
    artist: OnceCell<Artist>,
}

/// A track's API attributes.
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackAttributes {
    pub title: String,
    #[serde(default)]
    pub version: String,
    pub isrc: String,
    pub duration: String,
    pub copyright: String,
    pub explicit: bool,
    pub popularity: f32,
    pub availability: Vec<String>,
    pub media_tags: Vec<String>,
}

impl Track {
    /// Returns a new `Track` from a track's id.
    pub fn new(session: Arc<Session>, id: String) -> Result<Self, String> {
        let endpoint = format!("/tracks/{}", id);
        let mut data_json = session.get(&endpoint)?;
        let attributes_json = data_json["attributes"].take();

        let attributes: TrackAttributes = serde_json::from_value(attributes_json)
            .map_err(|e| format!("Unable to parse track API response: {}", e.to_string()))?;        

        Ok(Self {
            session,
            id,
            attributes,
            album: OnceCell::new(),
            artist: OnceCell::new(),
        })
    }

    /// Returns a reference to the `Album` associated with this track.
    /// 
    /// This `Album` is then cached within `self`.
    pub fn get_album(&self) -> Result<&Album, String> {
        self.album.get_or_try_init(|| -> Result<Album, String> {
            let album_relationships_endpoint = format!("/tracks/{}/relationships/albums", self.id);
            let data_json = self.session.get(&album_relationships_endpoint)?;
            let albums = data_json.as_array()
                .ok_or(String::from("Unable to parse album relationship API response"))?;

            // For now, we assume that there is only one album associated with a track.
            let album_json = albums.get(0)
                .ok_or(String::from("Unable to parse album relationship API response"))?;
            let album_id = album_json["id"]
                .as_str()
                .ok_or(String::from("Unable to parse album relationship API response"))?
                .to_string();
            
            let album = Album::new(Arc::clone(&self.session), album_id)?;
            Ok(album)
        })
    }

    /// Returns a reference to the `Artist` associated with this track.
    /// 
    /// This `Artist` is then cached within `self`.
    pub fn get_artist(&self) -> Result<&Artist, String> {
        self.artist.get_or_try_init(|| -> Result<Artist, String> {
            let artist_relationships_endpoint = format!("/tracks/{}/relationships/artists", self.id);
            let data_json = self.session.get(&artist_relationships_endpoint)?;
            let artists = data_json.as_array()
                .ok_or(String::from("Unable to parse artist relationship API response"))?;

            // For now, we assume that there is only one artist associated with a track.
            let artist_json = artists.get(0)
                .ok_or(String::from("Unable to parse artist relationship API response"))?;
            let artist_id = artist_json["id"]
                .as_str()
                .ok_or(String::from("Unable to parse artist relationship API response"))?
                .to_string();
            
            let artist = Artist::new(Arc::clone(&self.session), artist_id)?;
            Ok(artist)
        })
    }

    /// Returns a `Duration` corresponding this `Album`'s duration attribute.
    pub fn get_duration(&self) -> Duration {
        let re = Regex::new(r"^PT((?<hours>\d+)H)*((?<mins>\d+)M)*((?<secs>\d+)S)*$").unwrap();
        let Some(captures) = re.captures(&self.attributes.duration) else {
            return Duration::from_secs(0);
        };

        let hours: u64 = match captures.name("hours") {
            None => 0,
            Some(hours_match) => hours_match.as_str().parse().unwrap_or(0),
        };
        let mins: u64 = match captures.name("mins") {
            None => 0,
            Some(mins_match) => mins_match.as_str().parse().unwrap_or(0),
        };
        let secs: u64 = match captures.name("secs") {
            None => 0,
            Some(secs_match) => secs_match.as_str().parse().unwrap_or(0),
        };
    
        Duration::from_secs((hours * 60 * 60) + (mins * 60) + (secs))
    }

    /// Gets the url used for playback for this track.
    /// 
    /// Uses the unofficial Tidal API. 
    pub fn get_url(&self) -> Result<String, String> {
        let result = Python::with_gil(|py| -> PyResult<String> {
            let track = self.session.py_tidalapi_session.call_method1(py, "track", (&self.id,))?;
            track.call_method0(py, "get_url")?.extract(py)
        });

        match result {
            Err(err) => Err(format!("A Python exception occurred:\n{}", err.to_string())),
            Ok(track_url) => Ok(track_url),
        }
    }
}

/// A Tidal album.
#[derive(Debug)]
pub struct Album {
    session: Arc<Session>,
    pub id: String,
    pub attributes: AlbumAttributes,
    pub image_link: String,
}

/// An album's API attributes.
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlbumAttributes {
    pub title: String,
    pub barcode_id: String,
    pub number_of_volumes: u32,
    pub number_of_items: u32,
    pub duration: String,
    pub explicit: bool,
    pub release_date: String,
    pub copyright: String,
    pub popularity: f32,
    pub availability: Vec<String>,
    pub media_tags: Vec<String>,
}

impl Album {
    /// Returns a new `Album` from an album's id.
    pub fn new(session: Arc<Session>, id: String) -> Result<Self, String> {
        let endpoint = format!("/albums/{}", id);
        let mut data_json = session.get(&endpoint)?;
        let mut attributes_json = data_json["attributes"].take();

        let image_links_json = attributes_json["imageLinks"].take();
        // The first image link should be the highest res.
        let image_link_json = image_links_json.get(0)
                .ok_or(String::from("Unable to parse album API response"))?;
        let image_link = image_link_json["href"]
            .as_str()
            .ok_or(String::from("Unable to parse album API response"))?
            .to_string();

        let attributes: AlbumAttributes = serde_json::from_value(attributes_json)
            .map_err(|e| format!("Unable to parse album API response: {}", e.to_string()))?;        

        Ok(Self {
            session,
            id,
            attributes,
            image_link,
        })
    }

    /// Returns a `Duration` corresponding this `Album`'s duration attribute.
    pub fn get_duration(&self) -> Duration {
        let re = Regex::new(r"^PT((?<hours>\d+)H)*((?<mins>\d+)M)*((?<secs>\d+)S)*$").unwrap();
        let Some(captures) = re.captures(&self.attributes.duration) else {
            return Duration::from_secs(0);
        };

        let hours: u64 = match captures.name("hours") {
            None => 0,
            Some(hours_match) => hours_match.as_str().parse().unwrap_or(0),
        };
        let mins: u64 = match captures.name("mins") {
            None => 0,
            Some(mins_match) => mins_match.as_str().parse().unwrap_or(0),
        };
        let secs: u64 = match captures.name("secs") {
            None => 0,
            Some(secs_match) => secs_match.as_str().parse().unwrap_or(0),
        };

        Duration::from_secs((hours * 60 * 60) + (mins * 60) + (secs))
    }
}

/// A Tidal artist.
#[derive(Debug)]
pub struct Artist {
    session: Arc<Session>,
    pub id: String,
    pub attributes: ArtistAttributes,
}

/// An artist's API attributes.
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtistAttributes {
    pub name: String,
    pub popularity: f32,
}

impl Artist {
    /// Returns a new `Artist` from an artist's id.
    pub fn new(session: Arc<Session>, id: String) -> Result<Self, String> {
        let endpoint = format!("/artists/{}", id);
        let mut data_json = session.get(&endpoint)?;
        let attributes_json = data_json["attributes"].take();

        let attributes: ArtistAttributes = serde_json::from_value(attributes_json)
            .map_err(|e| format!("Unable to parse artist API response: {}", e.to_string()))?;        

        Ok(Self {
            session,
            id,
            attributes,
        })
    }
}
