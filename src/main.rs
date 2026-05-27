use std::process;

use color_eyre::Result;

use tidal_tui::App;

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;
    
    #[cfg(not(target_os = "macos"))]
    return run_tui().await;

    #[cfg(target_os = "macos")]
    return run_macos().await;
}

async fn run_tui() -> Result<()> {
    let mut app = tokio::task::spawn_blocking(|| {
        App::init()
        .unwrap_or_else(|e| {
            println!("{e}");
            process::exit(1);
        })
    }).await?;
    let mut terminal = ratatui::init();
    let result = app.run(&mut terminal);
    ratatui::restore();
    result
}

/// On macOS, souvlaki's media controls require AppKit's event loop to be
/// running on the main thread. We pump a headless winit event loop here
/// to satisfy that requirement, while the TUI runs on a Tokio worker thread.
#[cfg(target_os = "macos")]
async fn run_macos() -> Result<()> {
    use winit::application::ApplicationHandler;
    use winit::event::WindowEvent;
    use winit::event_loop::{ActiveEventLoop, EventLoop};
    use winit::platform::pump_events::{EventLoopExtPumpEvents, PumpStatus};
    use winit::window::WindowId;

    let mut event_loop = EventLoop::new()?;
    let proxy = event_loop.create_proxy();

    tokio::spawn(async move {
        run_tui().await.unwrap_or_else(|e| eprintln!("{e}"));
        let _ = proxy.send_event(());
    });

    struct NoopApp;
    impl ApplicationHandler for NoopApp {
        fn resumed(&mut self, _: &ActiveEventLoop) {}
        fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {}
        fn user_event(&mut self, event_loop: &ActiveEventLoop, _: ()) {
            event_loop.exit();
        }
    }

    let mut noop = NoopApp;
    loop {
        let status = event_loop.pump_app_events(None, &mut noop);
        if let PumpStatus::Exit(_) = status {
            break;
        }
    }

    Ok(())
}
