pub mod rtidalapi;

use std::{
    error::Error,
    sync::{Arc, mpsc, Mutex},
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
use rtidalapi::Track;
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
use tokio;

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
    queue: Vec<Track>,
    position: Duration,
    is_playing: bool,
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
            queue: vec![],
            position: Duration::from_secs(0),
            is_playing: false,
        })
    }

    /// Spawns another thread to poll for playback position updates and media control events.
    pub fn start_polling_thread(player: Arc<Mutex<Self>>) -> Result<(), Box<dyn Error>> {
        let (tx, rx) = mpsc::channel();

        {
            let mut locked_player = player.lock()
                .map_err(|e| format!("{e:#?}"))?;
            locked_player.controls.attach(move |event| { tx.send(event).unwrap(); })?;
        }

        thread::spawn(move || {
            loop {
                {
                    let mut locked_player = player.lock().unwrap();
                    if locked_player.is_playing {
                        let position = locked_player.sink.get_pos();
                        locked_player.position = position;
                        locked_player.controls.set_playback(MediaPlayback::Playing { progress: Some(MediaPosition(position)) }).unwrap();
                    }
                }
                
                if let Ok(event) = rx.try_recv() {
                    let mut locked_player = player.lock().unwrap();
                    let position = locked_player.position;

                    match event {
                        MediaControlEvent::Pause => {
                            locked_player.is_playing = false;
                            locked_player.controls.set_playback(MediaPlayback::Paused { progress: Some(MediaPosition(position)) }).unwrap_or_else(|e| println!("Can't set paused: {e:?}"));
                            locked_player.sink.pause();
                        },
                        MediaControlEvent::Play => {
                            locked_player.is_playing = true;
                            locked_player.controls.set_playback(MediaPlayback::Playing { progress: Some(MediaPosition(position)) }).unwrap();
                            locked_player.sink.play();
                        },
                        _ => {},
                    }
                }

                thread::sleep(Duration::from_millis(100));
            }
        });

        Ok(())
    }

    /// Sets this player's queue and clears the currently playing track, if one exists.
    pub fn set_queue(&mut self, tracks: Vec<Track>) {
        self.current_track = None;
        self.queue = tracks;
        self.sink.clear();
    }

    /// Randomly shuffles this player's queue.
    pub fn shuffle_queue(&mut self) {
        self.queue.shuffle(&mut rng());
    }

    /// Replaces the current track with the given `Track` and starts playback.
    pub fn play_new_track(&mut self, track: Track) -> Result<(), Box<dyn Error>> {
        let track_url = track.get_url()?;
        let track_attributes = track.get_attribtues()?;
        let album = track.get_album()?;

        let track_title = &track_attributes.title;
        let album_title = &album.attributes.title;
        let artist_name = &track.get_artist()?.attributes.name;
        let duration = track.get_duration()?;
        let cover_url = &album.image_link;

        self.sink.clear();

        self.controls.set_metadata(MediaMetadata {
            title: Some(track_title),
            album: Some(album_title),
            artist: Some(artist_name),
            duration: Some(duration),
            cover_url: Some(cover_url),
        })?;
        self.controls.set_playback(MediaPlayback::Playing { progress: None })?;
        self.is_playing = true;

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

        Ok(())
    }
}