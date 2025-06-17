use std::process;

use color_eyre::Result;

use tidal_tui::App;

fn main() -> Result<()> {
    color_eyre::install()?;
    let mut terminal = ratatui::init();
    let mut app = App::init()
        .unwrap_or_else(|e| {
            println!("{e}");
            process::exit(1);
        });
    let result = app.run(&mut terminal);
    ratatui::restore();
    result
}
