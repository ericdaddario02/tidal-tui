use std::{
    env,
    error::Error,
    sync::{Arc, Mutex}
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
    layout::Rect,
    style::Stylize,
    text::{Line, Text},
    widgets::{
        Block, BorderType, Padding, Paragraph, Widget
    },
    DefaultTerminal,
    Frame,
};

pub mod player;
pub mod rtidalapi;

use rtidalapi::{
    AudioQuality,
    Session,
    User,
};
use player::Player;

/// App state.
pub struct App {
    exit: bool,
    player: Arc<Mutex<Player>>,
    session: Arc<Session>,
    user: User,
}

impl App {
    /// Initializes a new app.
    pub fn init() -> Result<Self, Box<dyn Error>> {
        dotenv().ok();

        let session = Arc::new(
            Session::new(&env::var("TIDAL_CLIENT_ID")?, &env::var("TIDAL_CLIENT_SECRET")?)?
        );
        session.set_audio_quality(AudioQuality::High)?;

        let user = rtidalapi::User::get_current_user(Arc::clone(&session))?;

        let player = Arc::new(Mutex::new(Player::new()?));
        Player::start_polling_thread(Arc::clone(&player))?;

        Ok(Self {
            exit: false,
            player,
            session,
            user,
        })
    }

    /// Runs the application's main loop until the user quits.
    pub fn run(&mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        while !self.exit {
            terminal.draw(|frame| self.draw(frame))?;
            self.handle_events()?;
        }
        Ok(())
    }

    /// Draws a frame.
    fn draw(&self, frame: &mut Frame) {
        frame.render_widget(self, frame.area());
    }

    /// Handles user input events and updates application state accordingly.
    fn handle_events(&mut self) -> Result<()> {
        match event::read()? {
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

impl Widget for &App {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let title = Line::from(" tidal-tui ".bold());
        let keybinds = Line::from(vec![
            " Quit ".into(),
            "<Q> ".blue().bold(),
        ]);
        let block = Block::bordered()
            .title(title.centered())
            .title_bottom(keybinds.centered())
            .border_type(BorderType::Rounded)
            .padding(Padding::vertical(2));

        let counter_text = Text::from("Welcome");

        Paragraph::new(counter_text)
            .centered()
            .block(block)
            .render(area, buf);
    }
}
