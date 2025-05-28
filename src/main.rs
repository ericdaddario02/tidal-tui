use std::process;

use souvlaki;
use stream_download::{storage::{memory::MemoryStorageProvider}, Settings, StreamDownload};
use tokio;
use rodio::{Decoder, OutputStream, Sink};

use tidal_tui::rtidalapi;
use rtidalapi::{AudioQuality, Session};

#[tokio::main]
async fn main() {
    let session = Session::new_oauth().unwrap_or_else(|err| {
        println!("{err}");
        process::exit(1);
    });

    println!("{session:#?}");

    if let Err(err) = session.set_audio_quality(AudioQuality::High) {
        println!("{err}");
        process::exit(1);
    }

    let track_url = session.get_track_url(5120043).unwrap_or_else(|err| {
        println!("{err}");
        process::exit(1);
    });

    println!("{track_url}");

    // Setup audio player
    let (_stream, stream_handle) = OutputStream::try_default().unwrap();
    let sink = Sink::try_new(&stream_handle).unwrap();
    sink.set_volume(0.1);

    // Setup media controls
    let config = souvlaki::PlatformConfig {
        dbus_name: "tidal-tui",
        display_name: "tidal-tui",
        hwnd: None,
    };
    let mut controls = souvlaki::MediaControls::new(config).unwrap();
    controls.attach(|event| println!("Event received: {:?}", event)).unwrap();
    controls.set_metadata(souvlaki::MediaMetadata { 
        title: Some("this is the title"),
        album: Some("album yo"),
        artist: Some("best artist"),
        ..Default::default()
    }).unwrap();
    controls.set_playback(souvlaki::MediaPlayback::Playing {
        progress: Some(souvlaki::MediaPosition(std::time::Duration::from_secs(0)))
    }).unwrap();

    play_stream_example(&sink, &track_url).await;
    // play_file_example(&sink);
}

async fn play_stream_example(sink: &Sink, url: &String) {
    let reader = StreamDownload::new_http(
        url.parse().unwrap(),
        MemoryStorageProvider,
        Settings::default(),
    ).await.unwrap();

    let source = Decoder::new(reader).unwrap();
    sink.append(source);
    // sink.try_seek(std::time::Duration::from_secs(20)).unwrap();
    // sink.try_seek(std::time::Duration::from_secs(0)).unwrap();
    sink.sleep_until_end();
}

// fn play_stream_example2(sink: &Sink, url: &String) {
//     let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();

//     let future = async {
//         let reader = StreamDownload::new_http(
//             url.parse().unwrap(),
//             MemoryStorageProvider,
//             Settings::default(),
//         ).await.unwrap();

//         let source = Decoder::new(reader).unwrap();
//         sink.append(source);
//         // sink.try_seek(std::time::Duration::from_secs(20)).unwrap();
//         // sink.try_seek(std::time::Duration::from_secs(0)).unwrap();
//         sink.sleep_until_end();
//     };

//     rt.block_on(future);
// }

fn play_file_example(sink: &Sink) {
    let file = std::io::BufReader::new(std::fs::File::open("song.flac").unwrap());

    let source = Decoder::new(file).unwrap();
    sink.append(source);
    // sink.try_seek(std::time::Duration::from_secs(20)).unwrap();
    // sink.try_seek(std::time::Duration::from_secs(0)).unwrap();
    sink.sleep_until_end();
}
