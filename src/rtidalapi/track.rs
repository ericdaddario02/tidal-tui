use std::{
    collections::HashMap,
    sync::{
        Arc,
        Mutex,
    },
    time::Duration
};

use base64::{
    engine::general_purpose::STANDARD as BASE64, 
    Engine as _
};
use chrono::Utc;
use once_cell::sync::OnceCell;
use regex::Regex;
use serde::{Deserialize};
use serde_json;
use uuid::Uuid;

use super::Album;
use super::Artist;
use super::AudioQuality;
use super::Session;

/// A Tidal track.
#[derive(Clone, Debug)]
pub struct Track {
    session: Arc<Session>,
    pub id: String,

    // Cache the duration regex result.
    duration: OnceCell<Duration>,

    // The following fields are used to cache API results.
    attributes: OnceCell<TrackAttributes>,
    album: OnceCell<Album>,
    artist: OnceCell<Artist>,
    cached_manifest: Arc<Mutex<Option<CachedTrackManifest>>>,
    url_cache: Arc<Mutex<Option<(String, AudioQuality)>>>,
}

/// A track's API attributes.
#[allow(dead_code)]
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackAttributes {
    pub title: String,
    #[serde(default)]
    pub version: Option<String>,
    pub isrc: String,
    pub duration: String,
    #[serde(default)]
    pub copyright: HashMap<String, String>,
    pub explicit: bool,
    pub popularity: f32,
    pub availability: Vec<String>,
    pub media_tags: Vec<String>,
}

/// Normalization information used for both track and album normalization data.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NormalizationData {
    pub peak_amplitude: f32,
    pub replay_gain: f32,
}

/// A track's manifest.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackManifest {
    pub album_audio_normalization_data: NormalizationData,
    pub formats: Vec<String>,
    pub hash: String,
    pub preview_reason: Option<String>,
    pub track_audio_normalization_data: NormalizationData,
    pub track_presentation: String,
    pub uri: String,
}

/// Wrapper used for `TrackManifest` caching.
#[derive(Debug)]
struct CachedTrackManifest {
    manifest: TrackManifest,
    quality: AudioQuality,
    expires_at: i64,
}

impl Track {
    /// Returns a new `Track` from a track's id.
    pub fn new(session: Arc<Session>, id: String) -> Result<Self, String> {
        Ok(Self {
            session,
            id,
            duration: OnceCell::new(),
            attributes: OnceCell::new(),
            album: OnceCell::new(),
            artist: OnceCell::new(),
            cached_manifest: Arc::new(Mutex::new(None)),
            url_cache: Arc::new(Mutex::new(None)),
        })
    }

    /// Returns a reference to the `TrackAttributes` associated with this track.
    /// 
    /// This `TrackAttributes` is then cached within `self`.
    pub fn get_attribtues(&self) -> Result<&TrackAttributes, String> {
        self.attributes.get_or_try_init(|| -> Result<TrackAttributes, String> {
            let endpoint = format!("/tracks/{}", self.id);
            let mut data_json = self.session.get(&endpoint)?["data"].take();
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
            let data_json = self.session.get(&album_relationships_endpoint)?["data"].take();
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
            let data_json = self.session.get(&artist_relationships_endpoint)?["data"].take();
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

    /// Returns a reference to the `TrackManifest` associated with this track.
    /// 
    /// This `TrackManifest` is then cached within `self`.
    pub fn get_manifest(&self) -> Result<TrackManifest, String> {
        let mut cached_manifest = self.cached_manifest.lock().map_err(|e| format!("{e:#?}"))?;
        let quality = self.session.get_audio_quality();

        let is_missing = cached_manifest.is_none();
        let is_stale = cached_manifest.as_ref().is_some_and(|m| {
            m.quality != quality || m.expires_at <= Utc::now().timestamp()
        });

        if is_missing || is_stale {
            let mut endpoint = format!(
                "/trackManifests/{}?manifestType=MPEG_DASH&uriScheme=DATA&usage=PLAYBACK&adaptive=false",
                self.id
            );

            if quality >= AudioQuality::Low96 {
                endpoint.push_str("&formats=HEAACV1");
            }
            if quality >= AudioQuality::Low320 {
                endpoint.push_str("&formats=AACLC");
            }
            if quality >= AudioQuality::High {
                endpoint.push_str("&formats=FLAC");
            }
            if quality >= AudioQuality::Max {
                endpoint.push_str("&formats=FLAC_HIRES");
            }

            let playback_session_id = Uuid::new_v4().to_string();

            let headers = vec![
                ("x-playback-session-id", playback_session_id.as_str())
            ];

            let mut data_json = self.session.get_with_headers(&endpoint, headers)?
                ["data"].take();
            let attributes_json = data_json["attributes"].take();

            let mut manifest: TrackManifest = serde_json::from_value(attributes_json)
                .map_err(|e| format!("Unable to parse track manifest API response: {}", e.to_string()))?;

            let (_, encoded_xml) = manifest.uri.split_once(",")
                .ok_or("Unable to parse manifest XML")?;
            let decoded_xml = BASE64.decode(encoded_xml)
                .map_err(|e| format!("Unable to parse manifest XML: {}", e.to_string()))?;
            manifest.uri = String::from_utf8(decoded_xml)
                .map_err(|e| format!("Unable to parse manifest XML: {}", e.to_string()))?;
            
            let expires_at: i64 = manifest.uri
                .split("token=")
                .nth(1)
                .ok_or("Manifest URI has no expires_at")?
                .split('~')
                .next()
                .ok_or("Manifest URI has no expires_at")?
                .parse::<i64>()
                .map_err(|e| format!("Unable to parse track manifest expires_at: {}", e.to_string()))?;

            *cached_manifest = Some(CachedTrackManifest { manifest, quality, expires_at });
        }

        Ok(cached_manifest.as_ref().unwrap().manifest.clone())
    }

    /// Returns true if this Track already contains its attributes, album, and artist information.
    pub fn has_info(&self) -> bool {
        self.attributes.get().is_some() && self.album.get().is_some() && self.artist.get().is_some()
    }

    /// Returns a `Duration` corresponding this `Track`'s duration attribute.
    pub fn get_duration(&self) -> Result<&Duration, String> {
        self.duration.get_or_try_init(|| -> Result<Duration, String> {
            let re = Regex::new(r"^PT((?<hours>\d+)H)*((?<mins>\d+)M)*((?<secs>\d+)S)*$")
                .map_err(|e| format!("{}", e.to_string()))?;
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
        })
    }
}

#[cfg(feature = "unofficial")]
impl Track {
    /// Gets the url used for playback for this track.
    pub fn get_url(&self) -> Result<String, String> {
        let mut cache = self.url_cache.lock().map_err(|e| format!("{e:#?}"))?;
        let quality = self.session.get_audio_quality();

        if cache.as_ref().map(|(_, quality)| quality) != Some(&quality) {
            let endpoint = format!(
                "/tracks/{}/urlpostpaywall?audioquality={}&urlusagemode=STREAM&assetpresentation=FULL",
                self.id,
                quality.to_api_string(),
            );
            let json = self.session.get_unofficial(&endpoint)?;

            let url = json["urls"][0]
                .as_str()
                .ok_or(format!("Unable to get track url for track id {}", self.id))?
                .to_string();

            *cache = Some((url, quality));
        }

        Ok(cache.as_ref().unwrap().0.clone())
    }
}
