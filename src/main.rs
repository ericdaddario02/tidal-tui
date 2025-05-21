use std::process;

use tidal_lib::{AudioQuality, Session};

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
}
