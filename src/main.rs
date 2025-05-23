use std::process;
use rodio::{Decoder, OutputStream, Sink};

use tidal_tui::rtidalapi;
use rtidalapi::{AudioQuality, Session};

fn main() {
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

    let (_stream, stream_handle) = OutputStream::try_default().unwrap();
    let sink = Sink::try_new(&stream_handle).unwrap();
    sink.set_volume(0.1);

    let file = std::io::BufReader::new(std::fs::File::open("song.flac").unwrap());
    let source = Decoder::new(file).unwrap();
    sink.append(source);
    sink.try_seek(std::time::Duration::from_secs(20)).unwrap();
    sink.try_seek(std::time::Duration::from_secs(0)).unwrap();
    sink.sleep_until_end();
}
