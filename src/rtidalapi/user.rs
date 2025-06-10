use std::{
    sync::Arc,
};

use serde::{Deserialize};

use super::Session;

/// A Tidal user.
#[derive(Debug)]
pub struct User {
    session: Arc<Session>,
    pub id: String,
    pub attributes: UserAttributes,
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
        })
    }
}
