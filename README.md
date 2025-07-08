# tidal-tui

### A lightweight TUI client for the music streaming platform Tidal, written in Rust.

> [!IMPORTANT] 
> Not affiliated in any way with TIDAL. This is a third-party unofficial client.

## About the App

![Demo](.github/demo.gif)

Enjoy jamming out to your favourite lossless tracks right from your terminal using this fast, efficient, and lightweight TUI application built with Rust and [Ratatui](https://ratatui.rs/)!

Note: This application is still a work in progress, so not all Tidal features are implemented yet.

### Supported Features

- Supports playback up to 16-bit 44.1 kHz.
  - Available audio quality settings: Low (96 kbps), Low (320 kbps), High (16-bit 44.1 kHz).
- View and play all the tracks in your Collection.
- Playback controls through the terminal (Play/Pause, Next/Previous, Shuffle, Volume controls, etc.).
- OS media controls (*currently only supported on Linux*).

## Installation

### Build from Source

Note: On Linux systems, you may need to download the following development packages first:
- Packages for Ubuntu-based distros: `python3-dev`, `libasound2-dev`, `pkg-config`, `libssl-dev`
- If you are using another distro, check out the documentation for [`pyo3`](https://github.com/PyO3/pyo3), [`rodio`](https://github.com/RustAudio/rodio), and [`openssl`](https://docs.rs/openssl/latest/openssl/) to see which packages you may need for your specific distro.

The current version of this application requires a Python installation with the [`tidalapi`](https://github.com/EbbLabs/python-tidal) package installed. We recommend using a Python venv for `tidal-tui`, which can be created and used following the steps below.

1. Install [Rust](https://www.rust-lang.org/tools/install) (Edition 2024) on your system.
2. Install [Python](https://www.python.org/downloads/) on your system.
3. Clone the repository:
```
git clone git@github.com:ericdaddario02/tidal-tui.git
cd tidal-tui
```
4. Set up your Python venv:
```
python -m venv venv
source ./venv/bin/activate
pip install tidalapi
```
5. Build or run `tidal-tui`:
```
# Build the application.
# You can then copy the binary produced at ./target/release/tidal-tui and copy it to any other location (e.g. to a location in your $PATH).
cargo build --release

# Build and run the application all in one go.
cargo run --release
```

## Usage

To use `tidal-tui`, you must first have a [Tidal](https://tidal.com/) account.

You then need to register a Third-Party Application for your Tidal account:
1. Go to the [Tidal Developer Portal Dashboard](https://developer.tidal.com/dashboard).
2. Click `Create New App`.
3. Give it any name (e.g. `tidal-tui`) and click `Create App`.
4. In the `Settings` tab, click `Edit` and after selecting the following scopes, click `Save`:
    - `user.read`
    - `collection.write`
    - `collection.read`
    - `playlists.write`
    - `playlists.read` 
5. In the `Overview` tab, copy the `Client ID` and `Client Secret` and create the environment variables `TIDAL_CLIENT_ID` and `TIDAL_CLIENT_SECRET` on your system (e.g. in your .bashrc/.zshrc/etc., or within a `.env` file in the same directory as your `tidal-tui` binary).

Then go ahead and launch the application from wherever you placed the binary!
```
./tidal-tui
```

When you launch `tidal-tui` for the first time, you will have to login to Tidal twice using two links. This is because we need two different types of sessions for different sets of features.

The first link requires you to go to the Tidal login page and then paste the redirect link back into `tidal-tui`.
The second link requires you to go to the Tidal login page and click continue until it says a device is linked.

You only have to login the first time, so after this you can go ahead and enjoy using `tidal-tui`!

## Roadmap

The ideal goal is to add all the Tidal features you would expect in the GUI/web app.

The following are features that I would like to implement in the future, split between `tidal-tui` (the TUI client) and `rtidalapi` (my WIP Tidal Rust library):

### tidal-tui
- [ ] Toggle shuffle.
- [ ] Toggle repeat.
- [ ] Add track to queue.
- [ ] Play track next.
- [ ] Prefetch next song using a tokio task (so this doesn't block rendering).
- [ ] Add config file to save settings/options like volume, audio quality, etc.
- [ ] Start playing from a certain track (instead of only having options Shuffle All and Play All).
- [ ] Filter tracks (i.e. filter tracks in My Collection / Playlists / etc.).
- [ ] Desktop notifications (at least on Linux).
- [ ] My Collections - Albums tab.
	- [ ] Display and allow playing of tracks in an Album.
- [ ] My Collections - Artists tab.
	- [ ] Display all artist albums and allow playing from these albums.
	- [ ] Display all of an artist's tracks and allow playing from these.
- [ ] Tab for each playlist.
    - [ ] Display all their tracks and allow playing from these.
- [ ] Search (for tracks, albums, artists).
- [ ] Improve error displaying.
- [ ] Get OS media controls to work on MacOS and Windows.


### ritdalapi
- [ ] Remove dependence on py03/python.
	- [ ] Be able to get a track's playback info without using the `tidalapi` python package.
- [ ] Add support for MAX quality.
- [ ] Cache Album and Artist attributes like they are in Track.
- [ ] Continue implementing the rest of the API endpoints.
- [ ] Allow getting multiple tracks/albums/artists at once.
- [ ] Allow specifying includes when getting tracks, albums, artists.
- [ ] Add custom Error type(s) (that implements std::error::Error) and improve error handling.
- [ ] Develop an async version of this library.
- [ ] Move this library to be its own crate.
