use std::{
    sync::Arc,
    time::Duration
};

use once_cell::sync::OnceCell;
use regex::Regex;
use serde::{Deserialize};

use super::Session;

/// A Tidal album.
#[derive(Clone, Debug)]
pub struct Album {
    session: Arc<Session>,
    pub id: String,

    // Cache the duration regex result.
    duration: OnceCell<Duration>,

    pub attributes: AlbumAttributes,
    pub cover_art_url: String,
}

/// An album's API attributes.
#[allow(dead_code)]
#[derive(Clone, Debug, Deserialize)]
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
        let endpoint = format!("/albums/{}?include=coverArt", id);
        let mut json = session.get(&endpoint)?;

        let attributes_json = json["data"]["attributes"].take();
        let attributes: AlbumAttributes = serde_json::from_value(attributes_json)
            .map_err(|e| format!("Unable to parse album API response: {}", e.to_string()))?;

        let cover_art_url = json["included"]
            .get(0).ok_or(String::from("Unable to parse album API (cover art) response 1"))?  // We only include one thing (coverArt)
            ["attributes"]
            ["files"]
            .get(0).ok_or(String::from("Unable to parse album API (cover art) response 2"))?  // The first link is the highest res
            ["href"]
            .as_str().ok_or(String::from("Unable to parse album API (cover art) response 3"))?
            .to_string();

        Ok(Self {
            session,
            id,
            duration: OnceCell::new(),
            attributes,
            cover_art_url,
        })
    }

    /// Returns a `Duration` corresponding this `Album`'s duration attribute.
    pub fn get_duration(&self) -> Result<&Duration, String> {
        self.duration.get_or_try_init(|| -> Result<Duration, String> {
            let re = Regex::new(r"^PT((?<hours>\d+)H)*((?<mins>\d+)M)*((?<secs>\d+)S)*$")
                .map_err(|e| format!("{}", e.to_string()))?;
            let Some(captures) = re.captures(&self.attributes.duration) else {
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
        })
    }
}
