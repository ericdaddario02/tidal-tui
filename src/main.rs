use std::{
    env,
    process,
    sync::{Arc, Mutex}
};

use dotenv::dotenv;

use tidal_tui::{rtidalapi, Player};
use rtidalapi::{AudioQuality, Session};

fn main() {
    // Reads the .env file.
    dotenv().ok();

    Session::new(&env::var("TIDAL_CLIENT_ID").unwrap(), &env::var("TIDAL_CLIENT_SECRET").unwrap())
        .unwrap_or_else(|err| {
            println!("{err}");
            process::exit(1);
        });

    let session = Arc::new(Session::new_oauth().unwrap_or_else(|err| {
        println!("{err}");
        process::exit(1);
    }));

    println!("{session:#?}");
    return;

    if let Err(err) = session.set_audio_quality(AudioQuality::High) {
        println!("{err}");
        process::exit(1);
    }

    let track = rtidalapi::Track::new(Arc::clone(&session), "5120043".to_string()).unwrap_or_else(|err| {
        println!("{err}");
        process::exit(1);
    });

    println!("{track:#?}");

    let track_url = track.get_url().unwrap_or_else(|err| {
        println!("{err}");
        process::exit(1);
    });

    println!("{track_url}");

    let player = Arc::new(Mutex::new(Player::new().unwrap_or_else(|_err| {
        println!("Failed to create Player.");
        process::exit(1);
    })));

    Player::start_polling_thread(Arc::clone(&player)).unwrap();

    player.lock().unwrap().play_new_track(track).unwrap();

    loop {
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}
