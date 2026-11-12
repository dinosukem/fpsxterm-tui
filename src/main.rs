use {
    anyhow::{
        Context,
        Result,
    },
    crossterm::{
        event::EventStream,
        execute,
        terminal::{
            EnterAlternateScreen,
            LeaveAlternateScreen,
            disable_raw_mode,
            enable_raw_mode,
        },
    },
    fpsxterm_tui::{
        app::{
            App,
            Screen,
        },
        config::ConfigStore,
        event::Action,
        ssh::HostStore,
        ui,
    },
    futures_util::StreamExt,
    ratatui::{
        Terminal,
        backend::CrosstermBackend,
    },
    std::time::Duration,
    tokio::{
        sync::mpsc,
        time::interval,
    },
};

struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(
            std::io::stdout(),
            LeaveAlternateScreen,
            crossterm::event::DisableMouseCapture
        );
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("fpsxterm_tui=info".parse()?),
        )
        .with_writer(std::io::stderr)
        .init();

    let config_store = ConfigStore::discover()?;
    let config = config_store.load()?;
    let managed_dir = config_store.ssh_dir();
    let ssh_home = managed_dir
        .parent()
        .context("managed SSH directory has no parent")?
        .to_path_buf();
    let host_store = HostStore::new(&ssh_home, managed_dir);
    let hosts = host_store.list().unwrap_or_else(|error| {
        tracing::warn!(%error, "could not read SSH hosts at startup");
        Vec::new()
    });
    enable_raw_mode().context("enabling terminal raw mode")?;
    let _guard = TerminalGuard;
    if config.ui.mouse_enabled {
        execute!(
            std::io::stdout(),
            EnterAlternateScreen,
            crossterm::event::EnableMouseCapture
        )
    } else {
        execute!(std::io::stdout(), EnterAlternateScreen)
    }
    .context("initializing terminal screen")?;
    let backend = CrosstermBackend::new(std::io::stdout());
    let mut terminal = Terminal::new(backend)?;
    let (runtime_tx, runtime_rx) = mpsc::channel(64);
    run(
        &mut terminal,
        App::new(
            config,
            runtime_tx,
            host_store,
            hosts,
            config_store.history_path(),
            config_store,
        ),
        runtime_rx,
    )
    .await
}

async fn run(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    mut app: App,
    mut runtime_rx: mpsc::Receiver<fpsxterm_tui::terminal::RuntimeEvent>,
) -> Result<()> {
    let mut events = EventStream::new();
    let mut redraw = interval(Duration::from_millis(33));
    while !app.should_quit {
        tokio::select! {
            _ = redraw.tick() => {
                let size = terminal.size()?;
                let pty_area = ui::terminal_content_rect(ratatui::layout::Rect::new(0, 0, size.width, size.height));
                if app.screen == Screen::Terminal { app.resize_terminal(pty_area.height, pty_area.width); }
                for (id, status) in app.terminals.poll_exits() { app.record_terminal_exit(id, &status); }
                terminal.draw(|frame| { app.set_viewport(frame.area()); app.set_terminal_rect(ui::terminal_content_rect(frame.area())); ui::render(frame, &app) })?;
            }
            maybe_event = events.next() => match maybe_event {
                Some(Ok(event)) => fpsxterm_tui::input::handler::handle_event(&mut app, event),
                Some(Err(error)) => return Err(error.into()),
                None => app.update(Action::Quit),
            },
            runtime = runtime_rx.recv() => if let Some(event) = runtime { app.process_runtime_event(event); },
        }
    }
    Ok(())
}
