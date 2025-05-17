use pyo3::prelude::*;

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
            Err(err) => return Err(format!("A Python exception occurred:\n{}", err.to_string())),
            Ok(None) => return Err(String::from("Unable to login. Please try again later.")),
            Ok(Some(session)) => return Ok(session)
        }
    }
}
