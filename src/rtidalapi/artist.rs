use std::{
    sync::Arc,
};

use serde::{Deserialize};

use super::Session;

/// A Tidal artist.
#[derive(Clone, Debug)]
pub struct Artist {
    session: Arc<Session>,
    pub id: String,
    pub attributes: ArtistAttributes,
}

/// An artist's API attributes.
#[allow(dead_code)]
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtistAttributes {
    pub name: String,
    pub popularity: f32,
}

impl Artist {
    /// Returns a new `Artist` from an artist's id.
    pub fn new(session: Arc<Session>, id: String) -> Result<Self, String> {
        let endpoint = format!("/artists/{}", id);
        let mut data_json = session.get(&endpoint)?["data"].take();
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
