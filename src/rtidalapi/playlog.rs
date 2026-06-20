use std::sync::{Arc, Mutex};

use chrono::Utc;
use serde::Serialize;
use uuid::Uuid;

use super::{
    AudioQuality,
    Session,
};

/// The object responsible for tracking and sending PlayLog events to Tidal.
pub struct PlayLog {
    session: Arc<Session>,
    playback_session_payload: Mutex<PlaybackSessionPayload>,
}

#[allow(dead_code)]
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PlaybackSessionPayload {
    playback_session_id: String,
    requested_product_id: String,
    actual_product_id: String,
    product_type: String,
    source_type: String,
    source_id: String,
    start_timestamp: i64,
    end_timestamp: i64,
    start_asset_position: f64,
    end_asset_position: f64,
    actual_quality: String,
    actual_asset_presentation: String,
    actual_audio_mode: String,
    is_post_paywall: bool,
    actions: Vec<String>,
}

impl Default for PlaybackSessionPayload {
    fn default() -> Self {
        Self {
            playback_session_id: "".to_string(),
            requested_product_id: "".to_string(),
            actual_product_id: "".to_string(),
            product_type: "TRACK".to_string(),
            source_type: "".to_string(),
            source_id: "".to_string(),
            start_timestamp: 0,
            end_timestamp: 0,
            start_asset_position: 0.0,
            end_asset_position: 0.0,
            actual_quality: "".to_string(),
            actual_asset_presentation: "FULL".to_string(),
            actual_audio_mode: "STEREO".to_string(),
            is_post_paywall: true,
            actions: vec![],
        }
    }
}

impl PlayLog {
    /// Initialize and return a new `PlayLog`.
    pub fn new(session: Arc<Session>,) -> Self {
        PlayLog {
            session,
            playback_session_payload: Mutex::new(PlaybackSessionPayload::default()),
        }
    }

    /// Log that the track with the given `track_id` has started playing.
    /// 
    /// This function will clear any previously logged playback session,
    /// and must be called before calling `log_playback_end()`.
    pub fn log_playback_start(&self, playback_session_id: &str, track_id: &str, quality: AudioQuality) {
        let mut playback_session_payload = self.playback_session_payload.lock().unwrap();

        playback_session_payload.playback_session_id = playback_session_id.to_string();
        playback_session_payload.start_timestamp = Utc::now().timestamp_millis();
        playback_session_payload.requested_product_id = track_id.to_string();
        playback_session_payload.actual_product_id = track_id.to_string();
        playback_session_payload.actual_quality = quality.to_api_string();
    }

    /// Log that the current playback session has ended.
    /// 
    /// This function requires `log_playback_start()` to be called first.
    pub fn log_playback_end(&self, end_asset_position: f64) {
        let mut playback_session_payload = self.playback_session_payload.lock().unwrap();
    
        if playback_session_payload.playback_session_id.is_empty() {
            return;
        }

        playback_session_payload.end_asset_position = end_asset_position;
        playback_session_payload.end_timestamp = Utc::now().timestamp_millis();
    }

    /// Send the recorded playback_session event to Tidal.
    /// 
    /// Requires both `log_playback_start()` and `log_playback_end()` to have been previously called.
    pub fn send_playback_session(&self) -> Result<(), String> {
        let mut playback_session_payload = self.playback_session_payload.lock().unwrap();

        if playback_session_payload.end_asset_position == 0.0 {
            return Ok(());
        }

        let payload = playback_session_payload.clone();

        *playback_session_payload = PlaybackSessionPayload::default();
        drop(playback_session_payload);

        let event_id = Uuid::new_v4().to_string();
        let event_ts = Utc::now().timestamp_millis();

        let event = serde_json::json!({
            "group": "play_log",
            "name": "playback_session",
            "version": 2,
            "uuid": event_id,
            "ts": event_ts,
            "payload": payload,
        });

        dbg!(&payload);

        self.session.send_event(event)?;

        Ok(())
    }
}
