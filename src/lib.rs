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

use color_eyre::Result;
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
        Style,
        Stylize,
    },
    widgets::{
        Block,
        BorderType,
        Borders,
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
    AudioQuality,
    Session,
    Track,
    User,
};
use player::Player;

enum AppEvent {
    ReRender,
}

/// App state.
pub struct App {
    exit: bool,
    player: Arc<Mutex<Player>>,
    session: Arc<Session>,
    user: Arc<User>,
    rx: mpsc::UnboundedReceiver<AppEvent>,
    tx: mpsc::UnboundedSender<AppEvent>,
    collection_tracks: Arc<Mutex<Vec<Arc<Track>>>>,
    collection_tracks_fetched: Arc<AtomicBool>,
    collection_tracks_table_state: TableState,
}

impl App {
    /// Initializes a new app.
    pub fn init() -> Result<Self, Box<dyn Error>> {
        dotenv().ok();

        let session = Arc::new(
            Session::new(&env::var("TIDAL_CLIENT_ID")?, &env::var("TIDAL_CLIENT_SECRET")?).unwrap()
        );
        session.set_audio_quality(AudioQuality::High)?;

        let user = rtidalapi::User::get_current_user(Arc::clone(&session))?;

        let player = Arc::new(Mutex::new(Player::new()?));
        Player::start_polling_thread(Arc::clone(&player))?;

        let (tx, rx) = mpsc::unbounded_channel::<AppEvent>();

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
                Constraint::Length(8),
            ])
            .split(f.area());
        let main_area = main_layout[0];
        let now_playing_area = main_layout[1];

        // My Collection
        let my_collection_block = Block::new()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .title(" My Collection - Tracks ".bold());
        f.render_widget(my_collection_block, main_area);
        
        let inner_main_area = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Fill(1),
            ])
            .vertical_margin(1)
            .horizontal_margin(2)
            .split(main_area);

        self.draw_my_collections_tracks(f, inner_main_area[0]);

        // Now Playing
        let now_playing_block = Block::new()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .title(" Now Playing ".bold());
        f.render_widget(now_playing_block, now_playing_area);
    }

    /// Draws the My Collections - Tracks table.
    fn draw_my_collections_tracks(&mut self, f: &mut Frame, area: Rect) {
        if self.collection_tracks_fetched.load(Ordering::Relaxed) {
            let unlocked_collection_tracks = self.collection_tracks.lock().unwrap();
            let collection_tracks_rows: Vec<Row> = unlocked_collection_tracks
                .iter()
                .enumerate()
                .map(|(idx, track)| {
                    let current_position = self.collection_tracks_table_state.selected().unwrap_or(0);
                    let num_rows = area.height as usize;
                    let render_window_amount = num_rows + 10;

                    // Only render certain number of rows.
                    if idx >= current_position.saturating_sub(render_window_amount) && idx <= current_position.saturating_add(render_window_amount) {
                        if track.has_info() {
                            let number = (idx + 1).to_string();
                            let title = track.get_attribtues().unwrap().title.clone();
                            let artist = track.get_artist().unwrap().attributes.name.clone();
                            let album = track.get_album().unwrap().attributes.title.clone();
                            let duration = track.get_duration().unwrap();
                            let time = format!("{}:{}", duration.as_secs() / 60, duration.as_secs() % 60);

                            Row::new([number, title, artist, album, time])
                        } else {
                            let tx_clone = self.tx.clone();
                            let track_clone = Arc::clone(&track);

                            tokio::task::spawn_blocking(move || {
                                track_clone.get_attribtues().unwrap();
                                track_clone.get_artist().unwrap();
                                track_clone.get_album().unwrap();
                                tx_clone.send(AppEvent::ReRender).unwrap();
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

            f.render_stateful_widget(collection_tracks_table, area, &mut self.collection_tracks_table_state);
        } else {
            f.render_widget(Paragraph::new("Loading..."), area);

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
                tx_clone.send(AppEvent::ReRender).unwrap();
            });
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
}
