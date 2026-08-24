//! `omarchy-power` — a terminal view of what the laptop's hardware is doing.
//!
//! Reads only, for now. Writes go through `omarchy-powerd` over D-Bus once that
//! exists, so this binary never needs to run as root.

mod dump;
mod ui;

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use omarchy_power_core::{Backend, detect};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind};

/// How often the sensor snapshot is refreshed.
const REFRESH: Duration = Duration::from_secs(1);

fn main() -> Result<()> {
    match Cli::parse(std::env::args().skip(1))? {
        Cli::Tui => run_tui(),
        Cli::DumpFixture(dest) => dump::run(&dest),
        Cli::Help => {
            print_help();
            Ok(())
        }
    }
}

enum Cli {
    Tui,
    DumpFixture(PathBuf),
    Help,
}

impl Cli {
    fn parse(mut args: impl Iterator<Item = String>) -> Result<Self> {
        match args.next().as_deref() {
            None => Ok(Self::Tui),
            Some("dump-fixture") => Ok(Self::DumpFixture(
                args.next()
                    .map_or_else(|| PathBuf::from("omarchy-power-fixture"), PathBuf::from),
            )),
            Some("-h" | "--help" | "help") => Ok(Self::Help),
            Some(other) => anyhow::bail!("unknown command `{other}` (try --help)"),
        }
    }
}

fn print_help() {
    println!(
        "omarchy-power {}

USAGE:
    omarchy-power                     open the TUI
    omarchy-power dump-fixture [DIR]  capture this machine's sysfs attributes

ENVIRONMENT:
    OMARCHY_POWER_SYSFS   read from this directory instead of /sys",
        env!("CARGO_PKG_VERSION")
    );
}

fn run_tui() -> Result<()> {
    let backend = detect().context(
        "no supported hardware found\n\
         on MSI laptops this usually means the msi-ec module is missing: \
         install msi-ec-dkms and `modprobe msi_ec`",
    )?;

    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal, backend.as_ref());
    ratatui::restore();
    result
}

fn event_loop(terminal: &mut ratatui::DefaultTerminal, backend: &dyn Backend) -> Result<()> {
    let model = backend.model();
    let mut state = backend.read_state()?;
    let mut error: Option<String> = None;

    loop {
        terminal.draw(|frame| {
            ui::draw(
                frame,
                &ui::Screen {
                    backend: backend.name(),
                    model: model.as_deref(),
                    state: &state,
                    error: error.as_deref(),
                },
            );
        })?;

        // Polling doubles as the refresh timer: whatever happens first wins.
        if event::poll(REFRESH)?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            match key.code {
                KeyCode::Char('q' | 'Q') | KeyCode::Esc => return Ok(()),
                KeyCode::Char('r' | 'R') => {}
                _ => continue,
            }
        }

        // A failed read keeps the last good snapshot on screen; an EC that is
        // busy talking to the firmware returns errors now and then and the
        // display should not flicker because of it.
        match backend.read_state() {
            Ok(fresh) => {
                state = fresh;
                error = None;
            }
            Err(e) => error = Some(e.to_string()),
        }
    }
}
