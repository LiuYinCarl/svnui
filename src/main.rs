//! svnui — an SVN TUI client inspired by gitui.
//!
//! Entry point: terminal setup, event loop, and drawing. The event loop
//! multiplexes three channels (input, async svn results, spinner tick)
//! using `crossbeam_channel::select`, mirroring gitui's `main.rs`.

use clap::Parser;
use crossbeam_channel::{Receiver, select};
use crossterm::event::{self, Event};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::backend::{Backend, CrosstermBackend};
use std::io::{self, Stdout};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use svnui::app::App;
use svnui::components::Context;
use svnui::queue::Queue;
use svnui::svn;
use svnui::ui::style::Theme;

type Terminal = ratatui::Terminal<CrosstermBackend<Stdout>>;

const SPINNER_INTERVAL: Duration = Duration::from_millis(80);

/// Command line arguments.
#[derive(Parser, Debug)]
#[command(name = "svnui", version, about = "An SVN TUI client inspired by gitui")]
struct CliArgs {
    /// Working copy path (defaults to the current directory)
    #[arg(default_value = ".")]
    path: PathBuf,
}

/// Events multiplexed in the main loop.
enum QueueEvent {
    Input(Event),
    Async(svn::AsyncSvnNotification),
    Tick,
}

fn main() -> Result<(), String> {
    let args = CliArgs::parse();
    let cwd = std::fs::canonicalize(&args.path)
        .map_err(|e| format!("invalid path {}: {e}", args.path.display()))?;

    // channels
    let (tx_async, rx_async) = crossbeam_channel::unbounded::<svn::AsyncSvnNotification>();
    let (tx_tick, rx_tick) = crossbeam_channel::unbounded::<Instant>();
    let (tx_input, rx_input) = crossbeam_channel::unbounded::<Event>();
    let spinner_active = Arc::new(AtomicBool::new(false));

    // input thread: crossterm event::read blocks, so forward via channel
    std::thread::Builder::new()
        .name("input-reader".to_string())
        .spawn(move || {
            while let Ok(ev) = event::read() {
                if tx_input.send(ev).is_err() {
                    break;
                }
            }
        })
        .map_err(|e| format!("failed to spawn input thread: {e}"))?;

    // spinner ticker: only ticks while operations are pending
    {
        let spinner_active = spinner_active.clone();
        std::thread::Builder::new()
            .name("spinner-ticker".to_string())
            .spawn(move || {
                loop {
                    std::thread::sleep(SPINNER_INTERVAL);
                    if spinner_active.load(Ordering::Relaxed) {
                        let _ = tx_tick.send(Instant::now());
                    }
                }
            })
            .map_err(|e| format!("failed to spawn ticker: {e}"))?;
    }

    // terminal setup
    enable_raw_mode().map_err(|e| format!("failed to enable raw mode: {e}"))?;
    // a panic must not leave the terminal in raw mode / alternate screen
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        default_hook(info);
    }));
    let mut stdout = io::stdout();
    if let Err(e) = execute!(stdout, EnterAlternateScreen) {
        disable_raw_mode().ok();
        return Err(format!("failed to enter alternate screen: {e}"));
    }
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = match Terminal::new(backend) {
        Ok(t) => t,
        Err(e) => {
            disable_raw_mode().ok();
            let _ = execute!(io::stdout(), LeaveAlternateScreen);
            return Err(format!("terminal init: {e}"));
        }
    };
    terminal.hide_cursor().ok();

    let queue = Queue::new();
    let ctx = Context {
        queue: queue.clone(),
        theme: Theme::default(),
    };
    let svn = svn::Svn::new(cwd.clone(), tx_async);
    let mut app = App::new(cwd.clone(), svn, ctx);
    app.start();

    let result = run(
        &mut terminal,
        &mut app,
        &rx_input,
        &rx_async,
        &rx_tick,
        &spinner_active,
    );

    // teardown
    disable_raw_mode().ok();
    let _ = execute!(io::stdout(), LeaveAlternateScreen);

    if let Some(fatal) = app.fatal_error.clone() {
        eprintln!("{fatal}");
    }

    result
}

fn run<B: Backend>(
    terminal: &mut ratatui::Terminal<B>,
    app: &mut App,
    rx_input: &Receiver<Event>,
    rx_async: &Receiver<svn::AsyncSvnNotification>,
    rx_tick: &Receiver<Instant>,
    spinner_active: &AtomicBool,
) -> Result<(), String> {
    loop {
        spinner_active.store(app.pending > 0, Ordering::Relaxed);
        let ev = select_event(rx_input, rx_async, rx_tick)?;
        match ev {
            QueueEvent::Input(input) => {
                app.handle_input(&input)?;
                app.handle_queue_events();
                app.maybe_request_diff();
            }
            QueueEvent::Async(notif) => {
                app.handle_async(notif);
                app.handle_queue_events();
                app.maybe_request_diff();
            }
            QueueEvent::Tick => {
                app.tick();
            }
        }
        if app.quitting {
            break;
        }
        terminal
            .draw(|f| {
                let _ = app.draw(f);
            })
            .map_err(|e| format!("draw failed: {e}"))?;
    }
    Ok(())
}

fn select_event(
    rx_input: &Receiver<Event>,
    rx_async: &Receiver<svn::AsyncSvnNotification>,
    rx_tick: &Receiver<Instant>,
) -> Result<QueueEvent, String> {
    select! {
        recv(rx_input) -> msg => {
            msg.map(QueueEvent::Input).map_err(|e| format!("input channel closed: {e}"))
        }
        recv(rx_async) -> msg => {
            msg.map(QueueEvent::Async).map_err(|e| format!("async channel closed: {e}"))
        }
        recv(rx_tick) -> msg => {
            msg.map(|_| QueueEvent::Tick).map_err(|e| format!("tick channel closed: {e}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossbeam_channel::unbounded;
    use crossterm::event::KeyCode;
    use ratatui::backend::TestBackend;
    use svnui::app::App;
    use svnui::components::Context;
    use svnui::svn::models::StatusEntry;
    use svnui::test_support::TestRepo;

    #[test]
    fn cli_args_default_and_explicit() {
        let args = CliArgs::try_parse_from(["svnui"]).unwrap();
        assert_eq!(args.path, PathBuf::from("."));
        let args = CliArgs::try_parse_from(["svnui", "/tmp/repo"]).unwrap();
        assert_eq!(args.path, PathBuf::from("/tmp/repo"));
    }

    #[test]
    fn select_event_multiplexes_channels() {
        let (ti, ri) = unbounded::<Event>();
        let (ta, ra) = unbounded::<svn::AsyncSvnNotification>();
        let (tt, rt) = unbounded::<Instant>();

        // tick arrives
        tt.send(Instant::now()).unwrap();
        assert!(matches!(select_event(&ri, &ra, &rt), Ok(QueueEvent::Tick)));

        // async arrives
        ta.send(svn::AsyncSvnNotification::Status(Ok(vec![])))
            .unwrap();
        assert!(matches!(
            select_event(&ri, &ra, &rt),
            Ok(QueueEvent::Async(svn::AsyncSvnNotification::Status(_)))
        ));

        // input arrives
        ti.send(crossterm::event::Event::Key(
            crossterm::event::KeyEvent::new(
                KeyCode::Char('x'),
                crossterm::event::KeyModifiers::NONE,
            ),
        ))
        .unwrap();
        assert!(matches!(
            select_event(&ri, &ra, &rt),
            Ok(QueueEvent::Input(_))
        ));
    }

    #[test]
    fn run_loop_quits_on_q() {
        let Some(repo) = TestRepo::new() else { return };
        let (tx_async, rx_async) = unbounded();
        let (tx_input, rx_input) = unbounded::<Event>();
        let (_tx_tick, rx_tick) = unbounded::<Instant>();
        let spinner = AtomicBool::new(false);

        let queue = Queue::new();
        let ctx = Context {
            queue: queue.clone(),
            theme: Theme::default(),
        };
        let svn = svn::Svn::new(repo.wc.clone(), tx_async);
        let mut app = App::new(repo.wc.clone(), svn, ctx);
        app.handle_async(svn::AsyncSvnNotification::Info(Ok(())));
        app.handle_async(svn::AsyncSvnNotification::Status(Ok(vec![StatusEntry {
            status: 'M',
            props_status: ' ',
            tree_conflict: ' ',
            path: "Cargo.toml".to_string(),
            is_dir: false,
        }])));
        app.handle_async(svn::AsyncSvnNotification::Log(Ok(vec![])));

        let backend = TestBackend::new(120, 40);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal.hide_cursor().ok();

        // send a benign key first (triggers a draw), then quit
        tx_input
            .send(crossterm::event::Event::Key(
                crossterm::event::KeyEvent::new(
                    KeyCode::Char('j'),
                    crossterm::event::KeyModifiers::NONE,
                ),
            ))
            .unwrap();
        tx_input
            .send(crossterm::event::Event::Key(
                crossterm::event::KeyEvent::new(
                    KeyCode::Char('q'),
                    crossterm::event::KeyModifiers::NONE,
                ),
            ))
            .unwrap();

        let result = run(
            &mut terminal,
            &mut app,
            &rx_input,
            &rx_async,
            &rx_tick,
            &spinner,
        );
        assert!(result.is_ok(), "{result:?}");
        assert!(app.quitting);

        // the app rendered something before quitting
        let s: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(s.contains("Cargo.toml"), "{s}");
    }
}
