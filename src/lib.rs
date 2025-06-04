pub mod rtidalapi;

use std::error::Error;

use rodio::{
    Decoder,
    OutputStream,
    OutputStreamHandle,
    Sink
};
use rtidalapi::Track;
use souvlaki::{MediaControls, MediaMetadata};
use stream_download::{
    storage::{memory::MemoryStorageProvider},
    Settings,
    StreamDownload
};
use tokio;

/// Object responsible for playing audio and handling playback.
pub struct Player<'a> {
    _stream: OutputStream,
    _stream_handle: OutputStreamHandle,
    sink: Sink,
    tokio_rt: tokio::runtime::Runtime,
    controls: MediaControls,
    current_track: Option<Track<'a>>,
    queue: Vec<Track<'a>>,
}

impl<'a> Player<'a> {
    /// Set max volume because otherwise it is way too loud.
    const MAX_VOLUME: f32 = 0.15;

    /// Returns a new `Player`.
    pub fn new() -> Result<Self, Box<dyn Error>> {
        let tokio_rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;

        // Rodio setup.
        let (_stream, _stream_handle) = OutputStream::try_default()?;
        let sink = Sink::try_new(&_stream_handle)?;
        sink.set_volume(Self::MAX_VOLUME / 2.0);

        // Souvlaki setup.
        let config = souvlaki::PlatformConfig {
            dbus_name: "tidal-tui",
            display_name: "tidal-tui",
            hwnd: None,
        };
        let mut controls = souvlaki::MediaControls::new(config)
            .map_err(|e| format!("{e:#?}"))?;
        controls.attach(|event| { println!("Event received: {:?}", event)})
            .map_err(|e| format!("{e:#?}"))?;

        Ok(Self {
            _stream,
            _stream_handle,
            sink,
            tokio_rt,
            controls,
            current_track: None,
            queue: vec![],
        })
    }

    /// Replaces the current track with the given `Track` and starts playback.
    pub fn play_new_track(&mut self, track: Track<'a>) -> Result<(), Box<dyn Error>> {
        let track_url = track.get_url()?;

        self.sink.clear();

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

        let track_title = &track.attributes.title;
        let album_title = &track.get_album()?.attributes.title;

        self.controls.set_metadata(MediaMetadata {
            title: Some(track_title),
            album: Some(album_title),
            artist: Some("TODO"),
            duration: None,
            cover_url: None,
        })?;

        self.current_track = Some(track);

        Ok(())
    }
}