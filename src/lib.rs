use std::{
    env,
    error::Error,
    sync::{Arc, Mutex},
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
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::Stylize,
    widgets::{
        Block,
        BorderType,
        Borders,
        Widget,
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
    user: User,
    rx: mpsc::UnboundedReceiver<AppEvent>,
    tx: mpsc::UnboundedSender<AppEvent>,
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

        Ok(Self {
            exit: false,
            player,
            session,
            user,
            tx,
            rx,
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
    fn draw(&self, f: &mut Frame) {
        let main_layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Fill(1),
                Constraint::Length(8),
            ])
            .split(f.area());
        let main_area = main_layout[0];
        let now_playing_area = main_layout[1];

        let my_collection_block = Block::new()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .title(" My Collection ".bold());
        f.render_widget(my_collection_block, main_area);

        let now_playing_block = Block::new()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .title(" Now Playing ".bold());
        f.render_widget(now_playing_block, now_playing_area);
    }

    /// Handles user input events and updates application state accordingly.
    fn handle_terminal_event(&mut self, event: Event) -> Result<()> {
        match event {
            Event::Key(key_event) if key_event.kind == KeyEventKind::Press => {
                match key_event.code {
                    KeyCode::Char('q') => self.exit(),
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
}
