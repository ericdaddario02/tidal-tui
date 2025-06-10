use std::{
    sync::Arc,
    time::Duration
};

use regex::Regex;
use serde::{Deserialize};

use super::Session;

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
