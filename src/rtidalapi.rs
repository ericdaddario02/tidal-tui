use once_cell::unsync::OnceCell;
use pyo3::prelude::*;
use reqwest::blocking::Client;
use serde::Deserialize;
use serde_json::Value as JSONValue;

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

/// A currently logged in Tidal session.
#[derive(Debug)]
pub struct Session {
    access_token: String,
    country_code: String,
    request_client: Client,
    /// A reference to the tidalapi.Session Python object.
    py_tidalapi_session: PyObject
}

impl Session {
    /// Base URL of the public Tidal API.
    const BASE_URL: &str = "https://openapi.tidal.com/v2";

    /// Prints a login link for the user, and returns a `Session` instance upon successful login.
    /// 
    /// Returns an `Err` if: (1) a Python exception occurs, or (2) the login was unsuccessul.
    pub fn new_oauth() -> Result<Self, String> {
        pyo3::prepare_freethreaded_python();

        let result = Python::with_gil(|py| -> PyResult<Option<Self>> {
            let tidalapi = PyModule::import(py, "tidalapi")?;
            let session = tidalapi.call_method0("Session")?;

            let pathlib = PyModule::import(py, "pathlib")?;
            let path_type = pathlib.getattr("Path")?;
            let oauth_file_path = path_type.call1(("tidal-session-oauth.json",))?;

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

            Ok(Some(Self { 
                access_token: access_token.unwrap(), 
                country_code: country_code.unwrap(),
                request_client: Client::new(),
                py_tidalapi_session: session.unbind(),
            }))
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

    /// Makes a GET request to the public Tidal API.
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
}

/// A Tidal track.
#[derive(Debug)]
pub struct Track<'a> {
    session: &'a Session,
    pub id: String,

    #[allow(private_interfaces)]
    pub attributes: TrackAttributes,

    // Used for caching album API result.
    album: OnceCell<Album<'a>>,
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

impl<'a> Track<'a> {
    /// Returns a new `Track` from a track's id.
    pub fn new(session: &'a Session, id: String) -> Result<Self, String> {
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
        })
    }

    /// Gets the url used for playback for this track.
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
            
            let album = Album::new(self.session, album_id)?;
            Ok(album)
        })
    }
}

/// A Tidal album.
#[derive(Debug)]
pub struct Album<'a> {
    session: &'a Session,
    pub id: String,

    #[allow(private_interfaces)]
    pub attributes: AlbumAttributes,
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

impl<'a> Album<'a> {
    /// Returns a new `Album` from an album's id.
    pub fn new(session: &'a Session, id: String) -> Result<Self, String> {
        let endpoint = format!("/albums/{}", id);
        let mut data_json = session.get(&endpoint)?;
        let attributes_json = data_json["attributes"].take();

        let attributes: AlbumAttributes = serde_json::from_value(attributes_json)
            .map_err(|e| format!("Unable to parse album API response: {}", e.to_string()))?;        

        Ok(Self {
            session,
            id,
            attributes,
        })
    }
}
