//! `omarchy-power` — a terminal view of what the laptop's hardware is doing,
//! and the keys to change it.
//!
//! Never writes to sysfs itself: changes go to `omarchy-powerd` over D-Bus, so
//! this binary has no reason to run as root. Without the daemon it still opens,
//! read-only.

mod actions;
mod dump;
mod source;
mod ui;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use actions::Action;
use anyhow::Result;
use ratatui::crossterm::event::{self, Event, KeyEventKind};
use source::Source;
use ui::Status;

/// How often the sensor snapshot is refreshed.
const REFRESH: Duration = Duration::from_secs(1);
/// How long a message stays in the footer before the key hints come back.
const STATUS_LINGER: Duration = Duration::from_secs(4);

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

The TUI talks to omarchy-powerd over D-Bus. Without it, hardware is shown
read-only.

ENVIRONMENT:
    OMARCHY_POWER_SYSFS   read from this directory instead of /sys
                          (read-only mode only)",
        env!("CARGO_PKG_VERSION")
    );
}

fn run_tui() -> Result<()> {
    let source = Source::connect()?;
    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal, &source);
    ratatui::restore();
    result
}

/// The last thing that happened, and when — so it can be cleared on its own.
struct Footer {
    status: Option<Status>,
    since: Instant,
}

impl Footer {
    fn set(&mut self, status: Status) {
        self.status = Some(status);
        self.since = Instant::now();
    }

    fn expire(&mut self) {
        if self.since.elapsed() > STATUS_LINGER {
            self.status = None;
        }
    }
}

fn event_loop(terminal: &mut ratatui::DefaultTerminal, source: &Source) -> Result<()> {
    let backend = source.backend_name();
    let model = source.model();
    let capabilities = source.capabilities();
    let read_only = source.is_read_only();
    let charge_conflicts = source.charge_conflicts().to_vec();

    let mut state = source.snapshot()?;
    let mut footer = Footer {
        status: None,
        since: Instant::now(),
    };

    loop {
        terminal.draw(|frame| {
            ui::draw(
                frame,
                &ui::Screen {
                    backend: &backend,
                    model: model.as_deref(),
                    state: &state,
                    capabilities,
                    status: footer.status.as_ref(),
                    read_only,
                    charge_conflicts: &charge_conflicts,
                },
            );
        })?;

        // Polling doubles as the refresh timer: whatever happens first wins.
        if event::poll(REFRESH)?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            match actions::for_key(key.code, &state, capabilities) {
                Action::Quit => return Ok(()),
                Action::Ignored | Action::Refresh => {}
                Action::Unsupported(what) => {
                    footer.set(Status::failed(format!("this machine has no {what}")));
                }
                action => {
                    if let Some(status) = perform(source, action) {
                        footer.set(status);
                    }
                }
            }
        }

        // A failed read keeps the last good snapshot on screen; an EC that is
        // busy talking to the firmware returns errors now and then and the
        // display should not flicker because of it.
        match source.snapshot() {
            Ok(fresh) => state = fresh,
            Err(e) => footer.set(Status::failed(e.to_string())),
        }
        footer.expire();
    }
}

/// Carry out an action and describe the outcome.
fn perform(source: &Source, action: Action) -> Option<Status> {
    let (result, description) = match action {
        Action::SetPowerLevel(level) => (
            source.set_power_level(level),
            format!("power level: {level}"),
        ),
        Action::SetFanMode(mode) => (source.set_fan_mode(mode), format!("fan mode: {mode}")),
        Action::SetCoolerBoost(on) => (
            source.set_cooler_boost(on),
            format!("cooler boost: {}", on_off(on)),
        ),
        Action::SetBatterySaver(on) => (
            source.set_battery_saver(on),
            format!("battery saver: {}", on_off(on)),
        ),
        Action::SetChargeThreshold(percent) => (
            source.set_charge_end_threshold(percent),
            format!("charge limit: {percent}%"),
        ),
        // Handled by the caller; nothing to send anywhere.
        Action::Quit | Action::Refresh | Action::Ignored | Action::Unsupported(_) => return None,
    };

    Some(match result {
        Ok(()) => Status::ok(description),
        // The daemon's message is more specific than anything we could invent
        // here — a polkit refusal and a missing module read very differently.
        Err(e) => Status::failed(format!("{description} failed: {}", root_cause(&e))),
    })
}

fn on_off(on: bool) -> &'static str {
    if on { "on" } else { "off" }
}

/// D-Bus errors arrive wrapped; the innermost message is the useful one.
fn root_cause(error: &anyhow::Error) -> String {
    error
        .chain()
        .last()
        .map_or_else(|| error.to_string(), std::string::ToString::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_status_clears_itself_after_a_while() {
        let mut footer = Footer {
            status: None,
            since: Instant::now(),
        };
        footer.set(Status::ok("power level: balanced"));

        footer.expire();
        assert!(footer.status.is_some(), "should linger long enough to read");

        footer.since = Instant::now() - STATUS_LINGER - Duration::from_secs(1);
        footer.expire();
        assert!(footer.status.is_none(), "should not stay forever");
    }
}
