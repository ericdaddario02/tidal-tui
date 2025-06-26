use std::{
    env,
    error::Error,
    sync::{
        atomic::{
            AtomicBool,
            Ordering,
        },
        Arc,
        Mutex,
    },
    time::Duration,
};

use color_eyre::{
    eyre::eyre,
    Result,
};
use crossterm::event::{
    self,
    Event,
    KeyCode,
    KeyEventKind,
};
use dotenv::dotenv;
use ratatui::{
    layout::{Constraint,
        Direction,
        Layout,
        Rect,
    },
    style::{
        Color,
        Style,
        Stylize,
    },
    text::{
        Line, 
        Span,
    },
    widgets::{
        Block,
        BorderType,
        Borders,
        Gauge,
        Paragraph,
        Row,
        Table,
        TableState,
    },
    DefaultTerminal,
    Frame,
};
use tokio::sync::mpsc;

pub mod player;
pub mod rtidalapi;

use rtidalapi::{
    Session,
    Track,
    User,
};
use player::Player;

pub enum AppEvent {
    ReRender,
}

/// App state.
pub struct App {
    exit: bool,
    player: Arc<Mutex<Player>>,
    session: Arc<Session>,
    user: Arc<User>,
    rx: mpsc::Receiver<AppEvent>,
    tx: mpsc::Sender<AppEvent>,
    collection_tracks: Arc<Mutex<Vec<Arc<Track>>>>,
    collection_tracks_fetched: Arc<AtomicBool>,
    collection_tracks_table_state: TableState,
    is_shuffle: bool,
}

impl App {
    /// Initializes a new app.
    pub fn init() -> Result<Self, Box<dyn Error>> {
        dotenv().ok();

        let session = Arc::new(
            Session::new(&env::var("TIDAL_CLIENT_ID")?, &env::var("TIDAL_CLIENT_SECRET")?).unwrap()
        );

        let user = rtidalapi::User::get_current_user(Arc::clone(&session))?;

        // Set the AppEvent buffer to 2 to ignore multiple stored rerender events.
        const MAX_APP_EVENTS: usize = 2;

        let (tx, rx) = mpsc::channel::<AppEvent>(MAX_APP_EVENTS);
        let tx_clone = tx.clone();

        let player = Arc::new(Mutex::new(Player::new()?));
        Player::start_polling_thread(Arc::clone(&player), tx_clone)?;

        let collection_tracks_table_state = TableState::default();

        Ok(Self {
            exit: false,
            player,
            session,
            user: Arc::new(user),
            tx,
            rx,
            collection_tracks: Arc::new(Mutex::new(vec![])),
            collection_tracks_fetched: Arc::new(AtomicBool::new(false)),
            collection_tracks_table_state,
            is_shuffle: false,
        })
    }

    /// Runs the application's main loop until the user quits.
    pub fn run(&mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        while !self.exit {
            terminal.draw(|frame| self.draw(frame))?;

            loop {
                // Terminal events
                if event::poll(Duration::from_millis(100))? {
                    self.handle_terminal_event(event::read()?)?;
                    break;
                }

                // Internal app events
                if let Ok(app_event) = self.rx.try_recv() {
                    match app_event {
                        AppEvent::ReRender => break,
                    }
                }
            }
        }
        Ok(())
    }

    /// Draws a frame.
    fn draw(&mut self, f: &mut Frame) {
        let main_layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Fill(1),
                Constraint::Length(7),
            ])
            .split(f.area());
        let main_area = main_layout[0];
        let now_playing_area = main_layout[1];

        self.draw_my_collections_tracks(f, main_area);
        self.draw_now_playing(f, now_playing_area);
    }

    /// Draws the My Collections - Tracks table.
    fn draw_my_collections_tracks(&mut self, f: &mut Frame, area: Rect) {
        let my_collection_block = Block::new()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Color::Cyan)
            .title(" My Collection - Tracks ".bold())
            .title_bottom(Line::from(" <P>: Play  <S>: Shuffle ").right_aligned());
        f.render_widget(my_collection_block, area);
        
        let inner_area = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Fill(1),
            ])
            .vertical_margin(1)
            .horizontal_margin(2)
            .split(area)
            [0];

        if self.collection_tracks_fetched.load(Ordering::Relaxed) {
            let unlocked_collection_tracks = self.collection_tracks.lock().unwrap();
            let collection_tracks_rows: Vec<Row> = unlocked_collection_tracks
                .iter()
                .enumerate()
                .map(|(idx, track)| {
                    let current_position = self.collection_tracks_table_state.selected().unwrap_or(0);
                    let num_rows = inner_area.height as usize;
                    let render_window_amount = num_rows + 10;

                    // Only render certain number of rows.
                    if idx >= current_position.saturating_sub(render_window_amount) && idx <= current_position.saturating_add(render_window_amount) {
                        if track.has_info() {
                            let number = (idx + 1).to_string();
                            let title = track.get_attribtues().unwrap().title.clone();
                            let artist = track.get_artist().unwrap().attributes.name.clone();
                            let album = track.get_album().unwrap().attributes.title.clone();
                            let duration = track.get_duration().unwrap().clone();
                            let time = format_duration(duration);

                            Row::new([number, title, artist, album, time])
                        } else {
                            let tx_clone = self.tx.clone();
                            let track_clone = Arc::clone(&track);

                            tokio::task::spawn_blocking(move || {
                                track_clone.get_attribtues().unwrap();
                                track_clone.get_artist().unwrap();
                                track_clone.get_album().unwrap();
                                let _ = tx_clone.try_send(AppEvent::ReRender);
                            });

                            Row::new(["".to_string(), "".to_string(), "".to_string(), "".to_string(), "".to_string()])
                        }
                    } else {
                        Row::new(["".to_string(), "".to_string(), "".to_string(), "".to_string(), "".to_string()])
                    }
                })
                .collect();
            drop(unlocked_collection_tracks);

            let collection_tracks_table = Table::default()
                .header(
                    Row::new(["#", "Title", "Artist", "Album", "Time"])
                        .bottom_margin(1)
                )
                .widths([Constraint::Max(6), Constraint::Min(10), Constraint::Min(10), Constraint::Min(10), Constraint::Max(9)])
                .column_spacing(3)
                .rows(collection_tracks_rows)
                .row_highlight_style(Style::new().cyan().bold());

            f.render_stateful_widget(collection_tracks_table, inner_area, &mut self.collection_tracks_table_state);
        } else {
            f.render_widget(Paragraph::new("Loading..."), inner_area);

            let tx_clone = self.tx.clone();
            let collection_tracks_clone = Arc::clone(&self.collection_tracks);
            let collection_tracks_fetched_clone = Arc::clone(&self.collection_tracks_fetched);
            let user_clone = Arc::clone(&self.user);

            tokio::task::spawn_blocking(move || {
                let collection_tracks = user_clone.get_collection_tracks().unwrap().to_vec();
                {
                    *collection_tracks_clone.lock().unwrap() = collection_tracks
                        .into_iter()
                        .map(|t| Arc::new(t))
                        .collect();  
                }
                collection_tracks_fetched_clone.store(true, Ordering::Relaxed);
                let _ = tx_clone.try_send(AppEvent::ReRender);
            });
        }
    }

    /// Draws the now playing block.
    fn draw_now_playing(&mut self, f: &mut Frame, area: Rect) {
        let now_playing_block = Block::new()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Color::Cyan)
            .title(" Now Playing ".bold());
        f.render_widget(now_playing_block, area);

        let sections = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Fill(2),
                Constraint::Fill(3),
                Constraint::Fill(2),
            ])
            .vertical_margin(2)
            .horizontal_margin(2)
            .split(area);

        let left_layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(sections[0]);

        let middle_layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(sections[1]);
        let progress_layout = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(5),
                Constraint::Fill(1),
                Constraint::Length(5),
            ])
            .spacing(1)
            .split(middle_layout[2]);

        let right_layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(sections[2]);

        let unlocked_player = self.player.lock().unwrap(); 

        if let Some(current_track) = unlocked_player.get_current_track() {
            if current_track.has_info() {
                let track_title = current_track.get_attribtues().unwrap().title.clone();
                let artist_title = current_track.get_artist().unwrap().attributes.name.clone();
                let album_title = current_track.get_album().unwrap().attributes.title.clone();

                f.render_widget(Line::from(track_title.bold()), left_layout[0]);
                f.render_widget(Line::from(format!("{} - {}", artist_title, album_title)), left_layout[1]);
                f.render_widget(Line::from("Playing From: Tracks".dark_gray()), left_layout[2]);

                let shuffle_str = if self.is_shuffle { "Shuffle: On    " } else { "Shuffle: Off    " };
                let playing_status_str = if unlocked_player.is_playing() { "||" } else { "> " };
                
                f.render_widget(
                    Line::default().spans(
                        vec![
                            shuffle_str.dark_gray(),
                            playing_status_str.into(),
                            "    Repeat: Off".dark_gray(),
                        ]
                    ).centered(),
                    middle_layout[0]);

                let position = unlocked_player.get_position();
                let track_duration = current_track.get_duration().unwrap().clone();
                let position_progress = (position.as_secs() as f64) / (track_duration.as_secs() as f64);
                
                let progress_bar_label = Span::styled("", Color::LightCyan);
                let progress_bar = Gauge::default()
                    .gauge_style(Color::Cyan)
                    .on_dark_gray()
                    .ratio(position_progress)
                    .label(progress_bar_label);
                f.render_widget(Line::from(format_duration(position)).right_aligned(), progress_layout[0]);
                f.render_widget(progress_bar, progress_layout[1]);
                f.render_widget(Line::from(format_duration(track_duration)).left_aligned(), progress_layout[2]);

                let volume = unlocked_player.get_volume();
                let quality = self.session.get_audio_quality();

                f.render_widget(Line::from(format!("Volume: {}%", volume)).right_aligned(), right_layout[1]);
                f.render_widget(Line::from(format!("Quality: {}", quality.to_string())).right_aligned(), right_layout[2]);
            } else {
                f.render_widget(Line::from("Nothing playing").dark_gray(), left_layout[0]);
            }
        } else {
            f.render_widget(Line::from("Nothing playing").dark_gray(), left_layout[0]);
        }
    }

    /// Handles user input events and updates application state accordingly.
    fn handle_terminal_event(&mut self, event: Event) -> Result<()> {
        match event {
            Event::Key(key_event) if key_event.kind == KeyEventKind::Press => {
                match key_event.code {
                    KeyCode::Char('q') => self.exit(),
                    KeyCode::Up => self.prev_row(),
                    KeyCode::Down => self.next_row(),
                    KeyCode::Char('t') => self.go_to_top(),
                    KeyCode::Char('b') => self.go_to_bottom(),
                    KeyCode::Char('P') => self.play_all().map_err(|e| eyre!(format!("{e}")))?,
                    KeyCode::Char('S') => self.shuffle_all().map_err(|e| eyre!(format!("{e}")))?,
                    _ => {},
                }
            }
            _ => {},
        };
        Ok(())
    }

    /// Exit this application's main loop.
    fn exit(&mut self) {
        self.exit = true;
    }

    /// Selects the next row in the table.
    fn next_row(&mut self) {
        self.collection_tracks_table_state.select_next();
    }

    /// Selects the previous row in the table.
    fn prev_row(&mut self) {
        self.collection_tracks_table_state.select_previous();
    }

    /// Selects the first row in the table.
    fn go_to_top(&mut self) {
        self.collection_tracks_table_state.select_first();
    }

    /// Selects the last row in the table.
    fn go_to_bottom(&mut self) {
        self.collection_tracks_table_state.select_last();
    }

    /// Starts playing the collection's tracks from the beginning.
    fn play_all(&mut self) -> Result<(), Box<dyn Error>> {
        let collection_tracks_copy = self.user.get_collection_tracks()?.clone();

        let mut locked_player = self.player.lock()
            .map_err(|e| format!("{e:#?}"))?;
        locked_player.set_queue(collection_tracks_copy.into());
        drop(locked_player);

        let player_clone = Arc::clone(&self.player);
        tokio::task::spawn_blocking(move || {
            player_clone.lock().unwrap().play().unwrap();
        });

        self.is_shuffle = false;

        Ok(())
    }

    /// Starts playing the collection's tracks in a shuffled order.
    fn shuffle_all(&mut self) -> Result<(), Box<dyn Error>> {
        let collection_tracks_copy = self.user.get_collection_tracks()?.clone();

        let mut locked_player = self.player.lock()
            .map_err(|e| format!("{e:#?}"))?;
        locked_player.set_queue(collection_tracks_copy.into());
        locked_player.shuffle_queue();
        drop(locked_player);

        let player_clone = Arc::clone(&self.player);
        tokio::task::spawn_blocking(move || {
            player_clone.lock().unwrap().play().unwrap();
        });

        self.is_shuffle = true;

        Ok(())
    }
}

/// Formats a `Duration` into a `String` for displaying.
fn format_duration(duration: Duration) -> String {
    format!("{}:{:02}", (duration.as_secs_f64().round() as u64) / 60, (duration.as_secs_f64().round() as u64) % 60)
}
