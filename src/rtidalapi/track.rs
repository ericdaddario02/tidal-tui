use std::{
    collections::HashMap,
    sync::{
        Arc,
        Mutex,
    },
    time::Duration
};

use once_cell::sync::OnceCell;
use regex::Regex;
use serde::{Deserialize};
use serde_json;

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
    manifest: OnceCell<TrackManifest>,
    url: Arc<Mutex<Option<String>>>,
    url_audio_quality: Arc<Mutex<Option<AudioQuality>>>,
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
            manifest: OnceCell::new(),
            url: Arc::new(Mutex::new(None)),
            url_audio_quality: Arc::new(Mutex::new(None)),
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
    pub fn get_manifest(&self) -> Result<&TrackManifest, String> {
        self.manifest.get_or_try_init(|| -> Result<TrackManifest, String> {
            let mut endpoint = format!(
                "/trackManifests/{}?manifestType=MPEG_DASH&uriScheme=HTTPS&usage=PLAYBACK&adaptive=false",
                self.id
            );

            let quality = self.session.get_audio_quality();

            if quality >= AudioQuality::Low96 {
                endpoint.push_str("&formats=HEAACV1");
            }
            if quality >= AudioQuality::Low320 {
                endpoint.push_str("&formats=AACLC");
            }
            if quality >= AudioQuality::High {
                endpoint.push_str("&formats=FLAC");
            }
            // TODO: MAX quality not yet supported
            // if quality >= AudioQuality::Max {
            //     endpoint.push_str("&formats=FLAC_HIRES");
            // }

            let mut data_json = self.session.get(&endpoint)?["data"].take();
            let attributes_json = data_json["attributes"].take();

            let attributes: TrackManifest = serde_json::from_value(attributes_json)
                .map_err(|e| format!("Unable to parse track manifest API response: {}", e.to_string()))?;

            Ok(attributes)
        })
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
        let mut unlocked_url = self.url.lock().map_err(|e| format!("{e:#?}"))?;
        let mut unlocked_url_audio_quality = self.url_audio_quality.lock().map_err(|e| format!("{e:#?}"))?;

        if unlocked_url.is_none() || unlocked_url_audio_quality.is_some_and(|quality| quality != self.session.get_audio_quality()) {
            let endpoint = format!(
                "/tracks/{}/urlpostpaywall?audioquality={}&urlusagemode=STREAM&assetpresentation=FULL",
                self.id,
                self.session.get_audio_quality().to_api_string(),
            );
            let json = self.session.get_unofficial(&endpoint)?;

            let url = json["urls"][0]
                .as_str()
                .ok_or(format!("Unable to get track url for track id {}", self.id))?
                .to_string();

            *unlocked_url = Some(url);
            *unlocked_url_audio_quality = Some(self.session.get_audio_quality());
        }

        Ok(unlocked_url.as_ref().unwrap().clone())
    }
}
