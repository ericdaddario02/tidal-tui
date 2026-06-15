use std::{
    collections::VecDeque,
    error::Error,
    num::NonZero,
    sync::{
        mpsc,
        Arc,
        Mutex
    },
    thread,
    time::Duration
};

use dash_mpd::{MPD, parse};
use futures_util::StreamExt;
use rand::{
    seq::SliceRandom,
    rng
};
use rodio::{
    Decoder,
    DeviceSinkBuilder,
    MixerDeviceSink,
    Player as RodioPlayer
};
use souvlaki::{
    MediaControlEvent,
    MediaControls,
    MediaMetadata,
    MediaPlayback,
    MediaPosition,
    PlatformConfig
};
use stream_download::{
    async_read::AsyncReadStream,
    storage::memory::MemoryStorageProvider,
    Settings,
    StreamDownload
};
use tokio::{
    io::AsyncWriteExt,
    task::JoinHandle,
};

use crate::{
    rtidalapi::Track,
    AppEvent,
};

/// Wrapper for rodio MixerDeviceSink so Player can be Send+Sync.
struct MixerDeviceSinkWrapper(MixerDeviceSink);
unsafe impl Send for MixerDeviceSinkWrapper {}
unsafe impl Sync for MixerDeviceSinkWrapper {}
impl std::ops::Deref for MixerDeviceSinkWrapper {
    type Target = MixerDeviceSink;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Volume normalization mode.
pub enum NormalizationMode {
    None,
    Album,
    Track,
}

/// All the information we care about in the track manifests.
pub struct ParsedManifest {
    pub urls: Vec<String>,
    pub codec: String,
    pub sample_rate: u32,
    pub bit_depth: u32,
    pub content_length: u64,
}

/// Object responsible for playing audio and handling playback.
pub struct Player {
    output_stream: MixerDeviceSinkWrapper,
    sink: RodioPlayer,
    async_request_client: reqwest::Client,
    tokio_rt: tokio::runtime::Runtime,
    controls: MediaControls,
    current_track: Option<Arc<Track>>,
    queue: VecDeque<Arc<Track>>,
    queue_history: VecDeque<Arc<Track>>,
    position: Duration,
    is_playing: bool,
    volume: u32,
    normalization_mode: NormalizationMode,
    track_fetch_task_handle: Option<JoinHandle<()>>,

    // Information about the current track.
    replay_gain: f32,
    parsed_manifest: Option<ParsedManifest>,

    #[cfg(target_os = "windows")]
    /// Keeps the hidden window alive for the lifetime of the player.
    _hwnd_window: winit::window::Window,
}

impl Player {
    /// Set max volume for rodio because otherwise it is way too loud.
    const MAX_VOLUME: f32 = 0.5;

    /// Returns a new `Player`.
    pub fn new() -> Result<Self, Box<dyn Error>> {
        let tokio_rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .enable_all()
            .build()?;

        let builder = DeviceSinkBuilder::from_default_device()?
            .with_sample_rate(NonZero::new(44100).unwrap());

        #[cfg(target_os = "macos")]
        // Silence error messages when device sample rate changes.
        let builder = builder.with_error_callback(|_| {});
        
        let mut output_stream = builder.open_sink_or_fallback()?;
        output_stream.log_on_drop(false);

        let sink = RodioPlayer::connect_new(output_stream.mixer());
        sink.set_volume(Self::MAX_VOLUME / 2.0);

        #[cfg(not(target_os = "windows"))]
        let hwnd = None;

        #[cfg(target_os = "windows")]
        let (hwnd, hwnd_window) = Self::init_windows_hwnd();

        let config = PlatformConfig {
            dbus_name: "tidal-tui",
            display_name: "tidal-tui",
            hwnd,
        };
        let controls = MediaControls::new(config)?;

        Ok(Self {
            output_stream: MixerDeviceSinkWrapper(output_stream),
            sink,
            async_request_client: reqwest::Client::new(),
            tokio_rt,
            controls,
            current_track: None,
            queue: VecDeque::new(),
            queue_history: VecDeque::new(),
            position: Duration::from_secs(0),
            is_playing: false,
            volume: 50,
            normalization_mode: NormalizationMode::Track,
            track_fetch_task_handle: None,

            replay_gain: 0.0,
            parsed_manifest: None,

            #[cfg(target_os = "windows")]
            _hwnd_window: hwnd_window,
        })
    }

    fn open_new_output_stream(&mut self, sample_rate: u32) -> Result<(), Box<dyn Error>> {
        self.sink.stop();

        let builder = DeviceSinkBuilder::from_default_device()?
            .with_sample_rate(NonZero::new(sample_rate).unwrap());

        #[cfg(target_os = "macos")]
        // Silence error messages when device sample rate changes.
        let builder = builder.with_error_callback(|_| {});
        
        let mut output_stream = builder.open_sink_or_fallback()?;
        output_stream.log_on_drop(false);

        let sink = RodioPlayer::connect_new(output_stream.mixer());

        self.output_stream = MixerDeviceSinkWrapper(output_stream);
        self.sink = sink;

        Ok(())
    }

    /// Initializes an invisible window to allow Souvlaki to work on Windows.
    #[cfg(target_os = "windows")]
    fn init_windows_hwnd() -> (Option<*mut std::ffi::c_void>, winit::window::Window) {
        use winit::event_loop::EventLoop;
        use winit::platform::windows::EventLoopBuilderExtWindows;
        use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
        use winit::window::Window;

        let event_loop = EventLoop::builder()
            .with_any_thread(true)
            .build()
            .unwrap();

        let window = event_loop
            .create_window(Window::default_attributes().with_visible(false))
            .unwrap();

        let hwnd = match window.window_handle().unwrap().as_raw() {
            RawWindowHandle::Win32(handle) => handle.hwnd.get() as *mut std::ffi::c_void,
            _ => panic!("not running on Windows"),
        };

        (Some(hwnd), window)
    }

    /// Spawns another thread to poll for playback position updates and media control events.
    pub fn start_polling_thread(player: Arc<Mutex<Self>>, app_tx: tokio::sync::mpsc::Sender<AppEvent>) -> Result<(), Box<dyn Error>> {
        let (tx, rx) = mpsc::channel();

        {
            let mut unlocked_player = player.lock()
                .map_err(|e| format!("{e:#?}"))?;
            unlocked_player.controls.attach(move |event| { tx.send(event).unwrap(); })?;
        }

        thread::spawn(move || {
            loop {
                {
                    let mut unlocked_player = player.lock().unwrap();
                    if unlocked_player.is_playing {
                        let position = unlocked_player.sink.get_pos();

                        if unlocked_player.sink.empty() {
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
                        MediaControlEvent::Toggle => {
                            if unlocked_player.is_playing {
                                unlocked_player.pause().unwrap();
                            } else {
                                unlocked_player.play().unwrap();
                            }
                        },
                        _ => {},
                    }

                    let _ = app_tx.try_send(AppEvent::ReRender);
                }

                thread::sleep(Duration::from_millis(50));
            }
        });

        Ok(())
    }

    /// Returns a reference to the current track if one exists.
    pub fn get_current_track(&self) -> Option<&Arc<Track>> {
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
        self.volume = std::cmp::min(volume, 100);

        self.apply_volume_to_sink();
    }

    /// Returns this player's volume.
    pub fn get_volume(&self) -> u32 {
        self.volume
    }

    /// Returns this player's current ReplayGain value.
    pub fn get_replay_gain(&self) -> f32 {
        self.replay_gain
    }

    /// Returns this player's current `ParsedManifest` for the current track, if one exists.
    pub fn get_parsed_manifest(&self) -> Option<&ParsedManifest> {
        self.parsed_manifest.as_ref()
    }

    fn db_to_linear(db: f32) -> f32 {
        10f32.powf(db / 20.0)
    }

    /// Sets the rodio volume according to the user volume and the current replay gain.
    fn apply_volume_to_sink(&mut self) {
        let volume_ratio = (self.volume as f32) / 100.0;
        let linear_gain = Self::db_to_linear(self.replay_gain);

        self.sink.set_volume(Self::MAX_VOLUME * volume_ratio * linear_gain);
    }

    /// Sets this player's queue and clears the currently playing track, if one exists.
    pub fn set_queue(&mut self, tracks: Vec<Arc<Track>>) {
        self.current_track = None;
        self.queue = tracks.into();
        self.queue_history.clear();
        self.sink.clear();
    }

    /// Randomly shuffles this player's queue and queue history into a new queue.
    pub fn shuffle_queue(&mut self) {
        self.queue.append(&mut self.queue_history);
        self.queue.make_contiguous().shuffle(&mut rng());
    }

    /// Replaces the current track with the given `Track` and starts playback.
    pub fn play_new_track(&mut self, track: Arc<Track>) -> Result<(), Box<dyn Error>> {
        let track_attributes = track.get_attribtues()?;
        let album = track.get_album()?;

        let manifest = track.get_manifest()?;
        let parsed_manifest = Self::parse_manifest(&manifest.uri)?;

        let track_title = &track_attributes.title;
        let album_title = &album.attributes.title;
        let artist_name = &track.get_artist()?.attributes.name;
        let duration = track.get_duration()?.clone();
        let cover_url = &album.cover_art_url;

        if let Some(handle) = self.track_fetch_task_handle.take() {
            handle.abort();
        }
        self.sink.clear();

        if self.output_stream.config().sample_rate().get() != parsed_manifest.sample_rate {
            self.open_new_output_stream(parsed_manifest.sample_rate)?;
        }

        self.replay_gain = match self.normalization_mode {
            NormalizationMode::Album => manifest.album_audio_normalization_data.replay_gain,
            NormalizationMode::Track => manifest.track_audio_normalization_data.replay_gain,
            _ => 0.0,
        };
        self.apply_volume_to_sink();

        self.controls.set_metadata(MediaMetadata {
            title: Some(track_title),
            album: Some(album_title),
            artist: Some(artist_name),
            duration: Some(duration),
            cover_url: Some(cover_url),
        })?;
        self.controls.set_playback(MediaPlayback::Playing { progress: None })?;

        let (mut writer, reader) = tokio::io::duplex(512 * 1024);

        let client = self.async_request_client.clone();
        let urls = parsed_manifest.urls.clone();

        let handle = self.tokio_rt.spawn(async move {
            for url in urls {
                match client.get(&url).send().await {
                    Ok(resp) => {
                        let mut stream = resp.bytes_stream();
                        while let Some(chunk) = stream.next().await {
                            match chunk {
                                Ok(bytes) => { let _ = writer.write_all(&bytes).await; }
                                Err(e) => { eprintln!("Error: {e}"); break; }
                            }
                        }
                    }
                    Err(e) => { eprintln!("Error: {e}"); break; }
                }
            }
        });
        self.track_fetch_task_handle = Some(handle);

        let stream = self.tokio_rt.block_on(async {
            StreamDownload::from_stream(
                AsyncReadStream::new(reader, parsed_manifest.content_length),
                MemoryStorageProvider,
                Settings::default(),
            ).await
        })?;

        let source = Decoder::new_mp4(stream)?;
        self.sink.append(source);
        self.sink.play();

        self.current_track = Some(track);
        self.parsed_manifest = Some(parsed_manifest);
        self.is_playing = true;
        self.position = Duration::from_secs(0);

        // Prefetch the next track's info to reduce delay between tracks.
        if let Some(next_track) = self.queue.get(0) {
            let next_track = Arc::clone(next_track);

            self.tokio_rt.spawn_blocking(move || {
                let _ = next_track.get_attribtues();
                let _ = next_track.get_album();
                let _ = next_track.get_artist();
                let _ = next_track.get_manifest();
            });
        }

        Ok(())
    }

    /// Parses an MPEG DASH manifest and returns the urls and audio file information (codec, sample rate, bit depth).
    pub fn parse_manifest(xml: &str) -> Result<ParsedManifest, Box<dyn Error>> {
        let xml = regex::Regex::new(r#" group="[^"]*""#)?.replace_all(&xml, "").to_string();
        let mpd: MPD = parse(&xml)?;

        let mut urls = Vec::new();

        let period = &mpd.periods[0];
        let audio_set = period.adaptations.iter()
            .find(|a| {
                a.contentType.as_deref() == Some("audio")
                || a.mimeType.as_deref().is_some_and(|m| m.starts_with("audio"))
            })
            .ok_or("No audio adaptation set")?;

        let rep = audio_set.representations.iter()
            .max_by_key(|r| r.bandwidth.unwrap_or(0))
            .ok_or("No representations")?;

        let seg_template = rep.SegmentTemplate.as_ref()
            .or(audio_set.SegmentTemplate.as_ref())
            .ok_or("No SegmentTemplate")?;

        let rep_id = rep.id.as_deref().unwrap_or("0");
        let codec = rep.codecs.as_deref().unwrap_or("").to_string();
        let bandwidth = rep.bandwidth.unwrap_or(0);
        let start_number = seg_template.startNumber.unwrap_or(1) as u64;

        let mut rep_id_split = rep_id.split(',');

        let _ = rep_id_split.next().unwrap_or("");  // quality string
        let sample_rate: u32 = rep_id_split.next().unwrap_or("44100").parse()?;
        let bit_depth: u32 = rep_id_split.next().unwrap_or("16").parse()?;

        let duration_secs = mpd.mediaPresentationDuration
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        let content_length = ((duration_secs * (bandwidth as f64)) / 8.0) as u64;

        let resolve = |template: &str, number: u64, time: u64| -> String {
            template
                .replace("$RepresentationID$", rep_id)
                .replace("$Number$", &number.to_string())
                .replace("$Time$", &time.to_string())
                .replace("$Bandwidth$", &bandwidth.to_string())
        };

        let init_url = seg_template.initialization.as_deref()
            .map(|t| resolve(t, 0, 0))
            .ok_or("No initialization template")?;
        urls.push(init_url);

        let timeline = seg_template.SegmentTimeline.as_ref()
            .ok_or("No SegmentTimeline")?;

        let media_template = seg_template.media.as_deref()
            .ok_or("No media template")?;

        let mut number = start_number;
        let mut time: u64 = 0;

        for s in &timeline.segments {
            if let Some(t) = s.t { time = t as u64; }
            for _ in 0..=s.r.unwrap_or(0).max(0) {
                urls.push(resolve(media_template, number, time));
                number += 1;
                time += s.d as u64;
            }
        }

        Ok(ParsedManifest {
            urls,
            codec,
            sample_rate,
            bit_depth,
            content_length
        })
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
            // WORKAROUND: current rodio decoder creation does not allow backwards seeking
            // unless we allow a large delay on Decoder creation. So, this hack performs
            // backwards seeks by refetching and rebuilding the track's Decoder
            if position < self.sink.get_pos() {
                let track = self.current_track.take().unwrap();
                self.play_new_track(track)?;
            }

            self.sink.try_seek(position)?;
            self.position = self.sink.get_pos();
        }

        Ok(())
    }
}
