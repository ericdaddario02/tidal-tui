use pyo3::prelude::*;

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
    pub access_token: String,
    pub country_code: String,
    /// A reference to the tidalapi.Session Python object.
    pub py_tidalapi_session: PyObject
}

impl Session {
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
                py_tidalapi_session: session.unbind(),
            }))
        });

        match result {
            Err(err) => Err(format!("A Python exception occurred:\n{}", err.to_string())),
            Ok(None) => Err(String::from("Unable to login. Please try again later.")),
            Ok(Some(session)) => Ok(session),
        }
    }

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

    pub fn get_track_url(&self, track_id: u32) -> Result<String, String> {
        let result = Python::with_gil(|py| -> PyResult<String> {
            let track = self.py_tidalapi_session.call_method1(py, "track", (track_id,))?;
            track.call_method0(py, "get_url")?.extract(py)
        });

        match result {
            Err(err) => Err(format!("A Python exception occurred:\n{}", err.to_string())),
            Ok(track_url) => Ok(track_url),
        }
    }
}
