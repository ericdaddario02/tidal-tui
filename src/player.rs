use std::{
    collections::VecDeque,
    error::Error,
    sync::{
        mpsc,
        Arc,
        Mutex
    },
    thread,
    time::Duration
};

use rand::{
    seq::SliceRandom,
    rng
};
use rodio::{
    Decoder,
    OutputStream,
    OutputStreamHandle,
    Sink
};
use souvlaki::{
    MediaControlEvent,
    MediaControls,
    MediaMetadata,
    MediaPlayback,
    MediaPosition
};
use stream_download::{
    storage::memory::MemoryStorageProvider,
    Settings,
    StreamDownload
};

use crate::{
    rtidalapi::Track,
    AppEvent,
};

/// Wrapper for rodio OutputStream so Player can be Send+Sync.
struct PlayerOutputStreamWrapper {
    _stream: OutputStream,
}
unsafe impl Send for PlayerOutputStreamWrapper {}
unsafe impl Sync for PlayerOutputStreamWrapper {}

/// Object responsible for playing audio and handling playback.
pub struct Player {
    _stream: PlayerOutputStreamWrapper,
    _stream_handle: OutputStreamHandle,
    sink: Sink,
    controls: MediaControls,
    tokio_rt: tokio::runtime::Runtime,
    current_track: Option<Track>,
    queue: VecDeque<Track>,
    queue_history: VecDeque<Track>,
    position: Duration,
    is_playing: bool,
    volume: u32,
}

impl Player {
    /// Set max volume because otherwise it is way too loud.
    const MAX_VOLUME: f32 = 0.15;

    /// Returns a new `Player`.
    pub fn new() -> Result<Self, Box<dyn Error>> {
        let tokio_rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;

        let (_stream, _stream_handle) = OutputStream::try_default()?;
        let sink = Sink::try_new(&_stream_handle)?;
        sink.set_volume(Self::MAX_VOLUME / 2.0);

        let config = souvlaki::PlatformConfig {
            dbus_name: "tidal-tui",
            display_name: "tidal-tui",
            hwnd: None,
        };
        let controls = souvlaki::MediaControls::new(config)
            .map_err(|e| format!("{e:#?}"))?;

        Ok(Self {
            _stream: PlayerOutputStreamWrapper { _stream },
            _stream_handle,
            sink,
            tokio_rt,
            controls,
            current_track: None,
            queue: VecDeque::new(),
            queue_history: VecDeque::new(),
            position: Duration::from_secs(0),
            is_playing: false,
            volume: 50,
        })
    }

    /// Spawns another thread to poll for playback position updates and media control events.
    pub fn start_polling_thread(player: Arc<Mutex<Self>>, app_tx: tokio::sync::mpsc::Sender<AppEvent>) -> Result<(), Box<dyn Error>> {
        let (tx, rx) = mpsc::channel();

        {
            let mut unlocked_player = player.lock()
                .map_err(|e| format!("{e:#?}"))?;
            unlocked_player.controls.attach(move |event| { tx.send(event).unwrap(); })
                .map_err(|e| format!("{e:#?}"))?;
        }

        thread::spawn(move || {
            loop {
                {
                    let mut unlocked_player = player.lock().unwrap();
                    if unlocked_player.is_playing {
                        let position = unlocked_player.sink.get_pos();

                        if position != Duration::from_secs(0) && position == unlocked_player.position {
                            // Track is over.
                            unlocked_player.next().unwrap();
                            let _ = app_tx.try_send(AppEvent::ReRender);
                        } else {
                            if position.as_secs_f64().round() != unlocked_player.position.as_secs_f64().round() {
                                let _ = app_tx.try_send(AppEvent::ReRender);
                                unlocked_player.controls.set_playback(MediaPlayback::Playing { progress: Some(MediaPosition(position)) }).unwrap();
                            }
                            unlocked_player.position = position;
                        }
                    }
                }

                if let Ok(event) = rx.try_recv() {
                    let mut unlocked_player = player.lock().unwrap();

                    match event {
                        MediaControlEvent::Pause => {
                            unlocked_player.pause().unwrap();
                        },
                        MediaControlEvent::Play => {
                            unlocked_player.play().unwrap();
                        },
                        MediaControlEvent::Next => {
                            unlocked_player.next().unwrap();
                        },
                        MediaControlEvent::Previous => {
                            unlocked_player.prev().unwrap();
                        },
                        MediaControlEvent::SetPosition(MediaPosition(position)) => {
                            unlocked_player.set_position(position).unwrap();
                        },
                        _ => {},
                    }

                    let _ = app_tx.try_send(AppEvent::ReRender);
                }

                thread::sleep(Duration::from_millis(100));
            }
        });

        Ok(())
    }

    /// Returns a reference to the current track if one exists.
    pub fn get_current_track(&self) -> Option<&Track> {
        self.current_track.as_ref()
    }

    /// Returns the position of the current track.
    pub fn get_position(&self) -> Duration {
        self.position
    }

    /// Returns true iff this player is currently playing.
    pub fn is_playing(&self) -> bool {
        self.is_playing
    }

    /// Sets this player's volume satiatingly between 0 and 100.
    pub fn set_volume(&mut self, volume: u32) {
        if volume > 100 {
            self.volume = 100;
        } else {
            self.volume = volume;
        }

        let volume_ratio = (volume as f32) / 100.0;
        self.sink.set_volume(Self::MAX_VOLUME * volume_ratio);
    }

    /// Returns this player's volume.
    pub fn get_volume(&self) -> u32 {
        self.volume
    }

    /// Sets this player's queue and clears the currently playing track, if one exists.
    pub fn set_queue(&mut self, tracks: VecDeque<Track>) {
        self.current_track = None;
        self.queue = tracks;
        self.queue_history.clear();
        self.sink.clear();
    }

    /// Randomly shuffles this player's queue and queue history into a new queue.
    pub fn shuffle_queue(&mut self) {
        self.queue.append(&mut self.queue_history);
        self.queue.make_contiguous().shuffle(&mut rng());
    }

    /// Replaces the current track with the given `Track` and starts playback.
    pub fn play_new_track(&mut self, track: Track) -> Result<(), Box<dyn Error>> {
        let track_url = track.get_url()?;
        let track_attributes = track.get_attribtues()?;
        let album = track.get_album()?;

        let track_title = &track_attributes.title;
        let album_title = &album.attributes.title;
        let artist_name = &track.get_artist()?.attributes.name;
        let duration = track.get_duration()?.clone();
        let cover_url = &album.cover_art_url;

        self.sink.clear();

        self.controls.set_metadata(MediaMetadata {
            title: Some(track_title),
            album: Some(album_title),
            artist: Some(artist_name),
            duration: Some(duration),
            cover_url: Some(cover_url),
        })
            .map_err(|e| format!("{e:#?}"))?;
        self.controls.set_playback(MediaPlayback::Playing { progress: None })
            .map_err(|e| format!("{e:#?}"))?;

        let future = async {
            let reader = StreamDownload::new_http(
                track_url.parse()?,
                MemoryStorageProvider,
                Settings::default(),
            ).await?;

            let source = Decoder::new(reader)?;
            self.sink.append(source);
            self.sink.play();

            Ok::<(), Box<dyn Error>>(())
        };
        self.tokio_rt.block_on(future)?;

        self.current_track = Some(track);
        self.is_playing = true;
        self.position = Duration::from_secs(0);

        // Prefetch the next track's info to reduce delay between tracks.
        if let Some(next_track) = self.queue.get(0) {
            let _ = next_track.get_attribtues();
            let _ = next_track.get_album();
            let _ = next_track.get_artist();
            let _ = next_track.get_url();
        }

        Ok(())
    }

    /// Resumes playback if a track is paused, or starts playing the first track in the queue (if non-empty).
    pub fn play(&mut self) -> Result<(), Box<dyn Error>> {
        if self.current_track.is_some() && !self.is_playing {
            let position = self.position;
            self.is_playing = true;
            self.controls.set_playback(MediaPlayback::Playing { progress: Some(MediaPosition(position)) })?;
            self.sink.play();
        } else if self.current_track.is_none() && self.queue.len() > 0 {
            let track = self.queue.pop_front().unwrap();
            self.play_new_track(track)?;
        }

        Ok(())
    }

    /// Pauses playback is a track is playing.
    pub fn pause(&mut self) -> Result<(), Box<dyn Error>> {
        let position = self.position;
        self.is_playing = false;
        self.controls.set_playback(MediaPlayback::Paused { progress: Some(MediaPosition(position)) })?;
        self.sink.pause();

        Ok(())
    }

    /// Skips to playing the next track in the queue.
    pub fn next(&mut self) -> Result<(), Box<dyn Error>> {
        if let Some(current_track) = self.current_track.take() {
            if let Some(next_track) = self.queue.pop_front() {
                self.queue_history.push_back(current_track);
                self.play_new_track(next_track)?;
            } else {
                // No next tracks. Just start the same track over again (same as Tidal).
                self.current_track = Some(current_track);
                self.set_position(Duration::from_secs(0))?;
                self.controls.set_playback(MediaPlayback::Paused { progress: Some(MediaPosition(Duration::from_secs(0))) })?;
            }
        }

        Ok(())
    }

    /// Goes back to play the previous track in the queue history.
    pub fn prev(&mut self) -> Result<(), Box<dyn Error>> {
        if let Some(current_track) = self.current_track.take() {
            if let Some(prev_track) = self.queue_history.pop_back() {
                self.queue.push_front(current_track);
                self.play_new_track(prev_track)?;
            } else {
                // No previous tracks. Just start the same track over again (same as Tidal).
                self.current_track = Some(current_track);
                self.set_position(Duration::from_secs(0))?;
                self.controls.set_playback(MediaPlayback::Paused { progress: Some(MediaPosition(Duration::from_secs(0))) })?;
            }
        }

        Ok(())
    }

    /// Sets the position of playback in the player if there is a current track.
    pub fn set_position(&mut self, position: Duration) -> Result<(), Box<dyn Error>> {
        if self.current_track.is_some() {
            self.sink.try_seek(position)?;
            self.position = position;
        }

        Ok(())
    }
}
