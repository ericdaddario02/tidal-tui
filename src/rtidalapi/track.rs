use std::{
    sync::Arc,
    time::Duration
};

use once_cell::sync::OnceCell;
use pyo3::prelude::*;
use regex::Regex;
use serde::{Deserialize};
use serde_json;

use super::Album;
use super::Artist;
use super::Session;

/// A Tidal track.
#[derive(Debug)]
pub struct Track {
    session: Arc<Session>,
    pub id: String,

    // The following fields are used to cache API results.
    attributes: OnceCell<TrackAttributes>,
    album: OnceCell<Album>,
    artist: OnceCell<Artist>,
    url: OnceCell<String>,
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
    #[serde(default)]
    pub copyright: String,
    pub explicit: bool,
    pub popularity: f32,
    pub availability: Vec<String>,
    pub media_tags: Vec<String>,
}

impl Track {
    /// Returns a new `Track` from a track's id.
    pub fn new(session: Arc<Session>, id: String) -> Result<Self, String> {
        Ok(Self {
            session,
            id,
            attributes: OnceCell::new(),
            album: OnceCell::new(),
            artist: OnceCell::new(),
            url: OnceCell::new(),
        })
    }

    /// Returns a reference to the `TrackAttributes` associated with this track.
    /// 
    /// This `TrackAttributes` is then cached within `self`.
    pub fn get_attribtues(&self) -> Result<&TrackAttributes, String> {
        self.attributes.get_or_try_init(|| -> Result<TrackAttributes, String> {
            let endpoint = format!("/tracks/{}", self.id);
            let mut data_json = self.session.get(&endpoint)?;
            let attributes_json = data_json["attributes"].take();

            let attributes: TrackAttributes = serde_json::from_value(attributes_json)
                .map_err(|e| format!("Unable to parse track API response: {}", e.to_string()))?;

            Ok(attributes)
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

    /// Returns a `Duration` corresponding this `Track`'s duration attribute.
    pub fn get_duration(&self) -> Result<Duration, String> {
        let re = Regex::new(r"^PT((?<hours>\d+)H)*((?<mins>\d+)M)*((?<secs>\d+)S)*$").unwrap();
        let Some(captures) = re.captures(&self.get_attribtues()?.duration) else {
            return Ok(Duration::from_secs(0));
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
    
        Ok(Duration::from_secs((hours * 60 * 60) + (mins * 60) + (secs)))
    }

    /// Gets the url used for playback for this track.
    /// 
    /// Uses the unofficial Tidal API. 
    pub fn get_url(&self) -> Result<&String, String> {
        self.url.get_or_try_init(|| -> Result<String, String> {
            let result = Python::with_gil(|py| -> PyResult<String> {
                let track = self.session.py_tidalapi_session.call_method1(py, "track", (&self.id,))?;
                track.call_method0(py, "get_url")?.extract(py)
            });

            match result {
                Err(err) => Err(format!("A Python exception occurred:\n{}", err.to_string())),
                Ok(track_url) => Ok(track_url),
            }
        })
    }
}
