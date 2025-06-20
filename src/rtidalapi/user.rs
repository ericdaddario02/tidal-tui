use std::{
    sync::Arc,
};

use once_cell::sync::OnceCell;
use serde::{Deserialize};

use super::{
    Session,
    Track,
};

/// A Tidal user.
#[derive(Debug)]
pub struct User {
    session: Arc<Session>,
    pub id: String,
    pub attributes: UserAttributes,

    // The following fields are used to cache API results.
    collection_tracks: OnceCell<Vec<Track>>,
}

/// An user's API attributes.
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserAttributes {
    pub username: String,
    pub country: String,
    pub email: String,
    pub email_verified: bool,
}

impl User {
    /// Gets the currently logged in user from a session.
    pub fn get_current_user(session: Arc<Session>) -> Result<Self, String> {
        let endpoint = "/users/me";
        let mut data_json = session.get(&endpoint)?;

        let id = data_json["id"].as_str()
            .ok_or(String::from("Unable to get current user"))?
            .to_string();

        let attributes_json = data_json["attributes"].take();
        let attributes: UserAttributes = serde_json::from_value(attributes_json)
            .map_err(|e| format!("Unable to parse user API response: {}", e.to_string()))?;

        Ok(Self {
            session,
            id,
            attributes,
            collection_tracks: OnceCell::new(),
        })
    }

    /// Returns a list of tracks in the user's collection.
    pub fn get_collection_tracks(&self) -> Result<&Vec<Track>, String> {
        self.collection_tracks.get_or_try_init(|| -> Result<Vec<Track>, String> {
            let endpoint = format!("/users/{}/favorites/tracks?limit=10000", self.id);
            let res_json = self.session.get_unofficial(&endpoint)?;

            let size = res_json["totalNumberOfItems"]
                .as_u64()
                .ok_or(String::from("Unable to get collection tracks"))?;

            let mut collection_tracks: Vec<Track> = Vec::with_capacity(size as usize);

            let items_array = res_json["items"]
                .as_array()
                .ok_or(String::from("Unable to get collection tracks"))?;

            for json in items_array {
                let track_id = json["item"]["id"]
                    .as_u64()
                    .ok_or(String::from("Unable to get collection tracks"))?
                    .to_string();
                let track = Track::new(Arc::clone(&self.session), track_id)?;
                collection_tracks.push(track);
            }

            Ok(collection_tracks)
        })
    }
}
