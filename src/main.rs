use std::process;

use tidal_tui::{rtidalapi, Player};
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

    let track = rtidalapi::Track::new(&session, 5120043);

    let track_url = track.get_url().unwrap_or_else(|err| {
        println!("{err}");
        process::exit(1);
    });

    println!("{track_url}");

    let mut player = Player::new().unwrap_or_else(|_err| {
        println!("Failed to create Player.");
        process::exit(1);
    });

    player.play_new_track(track).unwrap();

    loop {}
}
