use std::process;

use tidal_lib::Session;

fn main() {
    let session = Session::new_oauth().unwrap_or_else(|err| {
        println!("{err}");
        process::exit(1);
    });

    println!("{session:#?}");
}
