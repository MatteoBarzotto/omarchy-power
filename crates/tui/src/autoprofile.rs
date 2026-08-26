//! Following the window you are actually working in.
//!
//! MSI Center calls this an AI mode; there is nothing intelligent about it and
//! nothing vendor-specific either. A rule says which window class deserves
//! which power profile, and something has to watch the compositor and act.
//!
//! Two decisions shape the whole module.
//!
//! **It runs in your session, not in the daemon.** The compositor's socket
//! lives in the user's runtime directory, and a root daemon reaching into it
//! would mean either loosening the sandbox or guessing which session to follow
//! on a machine with two. Here the watcher is an ordinary unprivileged process
//! that talks to the bus like any other client.
//!
//! **It holds a power-profiles-daemon profile rather than setting hardware.**
//! PPD stays the source of truth for what profile the machine is in — that is
//! the decision the whole project is built on — so a hold is picked up by our
//! daemon, by the desktop's own indicator, and by anything else listening. A
//! hold also ends by itself if this process dies, which a direct write would
//! not: a crashed watcher must not leave a laptop pinned to `performance`.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

#[zbus::proxy(
    interface = "org.freedesktop.UPower.PowerProfiles",
    default_service = "org.freedesktop.UPower.PowerProfiles",
    default_path = "/org/freedesktop/UPower/PowerProfiles"
)]
trait PowerProfiles {
    /// Returns a cookie identifying the hold, which is what releases it again.
    fn hold_profile(&self, profile: &str, reason: &str, application_id: &str) -> zbus::Result<u32>;
    fn release_profile(&self, cookie: u32) -> zbus::Result<()>;
}

/// Where the rules live, under the user's config directory.
const CONFIG: &str = "omarchy-power/autoprofile.toml";

/// What PPD is told when asked why the profile is held.
const REASON: &str = "active window";
const APP_ID: &str = "org.omarchy.Power1.autoprofile";

/// A window class mapped to the profile it deserves.
///
/// A plain table rather than an ordered list of rules: order in a config file
/// is invisible to the person editing it, so specificity decides instead —
/// see [`Rules::profile_for`].
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rules {
    /// What to hold when nothing matches. Omitted means "hold nothing", which
    /// leaves whatever the user or their desktop chose alone.
    #[serde(default)]
    default: Option<String>,
    #[serde(default)]
    classes: BTreeMap<String, String>,
}

impl Rules {
    pub fn parse(text: &str) -> Result<Self> {
        toml::from_str(text).context("parsing the autoprofile rules")
    }

    pub fn load() -> Result<Self> {
        let path = config_path()?;
        match std::fs::read_to_string(&path) {
            Ok(text) => Self::parse(&text).with_context(|| format!("in {}", path.display())),
            // No rules file is not a failure: it is how the feature stays off
            // until someone asks for it.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => bail!(
                "no rules at {}\n\
                 write one first — see the autoprofile section of the README",
                path.display()
            ),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    /// The profile for a window class, or the default.
    ///
    /// A key ending in `*` matches any class starting with the rest of it, and
    /// the longest match wins. That way `steam_app_*` and `steam_app_570` can
    /// both appear and the specific one decides, whatever order they sit in.
    pub fn profile_for(&self, class: &str) -> Option<&str> {
        let best = self
            .classes
            .iter()
            .filter(|(pattern, _)| matches(pattern, class))
            .max_by_key(|(pattern, _)| pattern.trim_end_matches('*').len())
            .map(|(_, profile)| profile.as_str());
        best.or(self.default.as_deref())
    }
}

fn matches(pattern: &str, class: &str) -> bool {
    match pattern.strip_suffix('*') {
        Some(prefix) => class.starts_with(prefix),
        None => pattern == class,
    }
}

fn config_path() -> Result<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .context("neither XDG_CONFIG_HOME nor HOME is set")?;
    Ok(base.join(CONFIG))
}

/// This session's compositor directory, holding both its sockets.
fn hypr_dir() -> Result<PathBuf> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").context("XDG_RUNTIME_DIR is not set")?;
    let signature = std::env::var("HYPRLAND_INSTANCE_SIGNATURE")
        .context("HYPRLAND_INSTANCE_SIGNATURE is not set; is this a Hyprland session?")?;
    Ok(PathBuf::from(runtime).join("hypr").join(signature))
}

/// The class of the window focused right now.
///
/// Without this the watcher would sit idle until the next window switch, which
/// makes starting it look like it did nothing — the common case being a login
/// script that starts it while a game is already open.
fn current_class(dir: &Path) -> Option<String> {
    let path = dir.join(".socket.sock");
    let mut stream = UnixStream::connect(&path).ok()?;
    stream.write_all(b"activewindow").ok()?;
    let mut reply = String::new();
    stream.read_to_string(&mut reply).ok()?;
    class_in_reply(&reply).map(str::to_owned)
}

/// Pull the class out of the compositor's reply to `activewindow`.
///
/// The reply is an indented block of `key: value` lines. `initialClass` is
/// deliberately not accepted: a window that changed its class since mapping
/// should be matched on what it is now.
fn class_in_reply(reply: &str) -> Option<&str> {
    reply
        .lines()
        .map(str::trim)
        .find_map(|line| line.strip_prefix("class: "))
        .filter(|class| !class.is_empty())
}

/// The window class in an `activewindow` event, if the line is one.
///
/// Hyprland writes `activewindow>>class,title`, and a title may contain commas
/// — the class is everything up to the first one. An empty class means the last
/// window closed and nothing has focus.
fn active_class(line: &str) -> Option<&str> {
    let payload = line.strip_prefix("activewindow>>")?;
    Some(payload.split(',').next().unwrap_or(""))
}

/// Watch the compositor and hold a profile to match, until killed.
pub fn run() -> Result<()> {
    let rules = Rules::load()?;
    let dir = hypr_dir()?;
    let connection =
        zbus::blocking::Connection::system().context("connecting to the system bus")?;
    let ppd = PowerProfilesProxyBlocking::new(&connection)
        .context("power-profiles-daemon is not on the bus")?;

    // Subscribe before asking what is focused now, so a switch happening in
    // between arrives as an event instead of being missed.
    let events = UnixStream::connect(dir.join(".socket2.sock"))
        .with_context(|| format!("connecting to {}", dir.display()))?;
    let mut hold = Hold::new(&ppd);
    eprintln!("watching the active window; holding profiles through power-profiles-daemon");

    if let Some(class) = current_class(&dir) {
        hold.follow(rules.profile_for(&class));
    }
    for line in BufReader::new(events).lines() {
        let line = line.context("reading compositor events")?;
        if let Some(class) = active_class(&line) {
            hold.follow(rules.profile_for(class));
        }
    }
    Ok(())
}

/// The one profile hold this watcher owns, moved from profile to profile.
struct Hold<'a> {
    ppd: &'a PowerProfilesProxyBlocking<'a>,
    /// The cookie PPD gave us, and which profile it is holding.
    current: Option<(u32, String)>,
}

impl<'a> Hold<'a> {
    fn new(ppd: &'a PowerProfilesProxyBlocking<'a>) -> Self {
        Self { ppd, current: None }
    }

    /// Hold `wanted`, releasing whatever was held before. `None` releases and
    /// holds nothing, leaving the profile to the user and their desktop.
    fn follow(&mut self, wanted: Option<&str>) {
        if wanted == self.current.as_ref().map(|(_, profile)| profile.as_str()) {
            return;
        }
        // Release first: two holds of different profiles would leave PPD
        // resolving between them, and the older one is no longer wanted.
        if let Some((cookie, _)) = self.current.take()
            && let Err(e) = self.ppd.release_profile(cookie)
        {
            eprintln!("releasing the previous hold: {e}");
        }
        let Some(profile) = wanted else {
            return;
        };
        match self.ppd.hold_profile(profile, REASON, APP_ID) {
            Ok(cookie) => self.current = Some((cookie, profile.to_owned())),
            // A profile this machine does not have is a rule worth fixing, not
            // a reason to stop watching.
            Err(e) => eprintln!("holding {profile}: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules() -> Rules {
        Rules::parse(
            r#"
            default = "balanced"

            [classes]
            "steam_app_*" = "performance"
            "steam_app_570" = "power-saver"
            "foot" = "power-saver"
            "#,
        )
        .unwrap()
    }

    #[test]
    fn an_exact_class_wins_over_the_pattern_that_also_matches() {
        // Both rules match; the specific one has to decide, and it must not
        // depend on which line came first in the file.
        assert_eq!(rules().profile_for("steam_app_570"), Some("power-saver"));
        assert_eq!(rules().profile_for("steam_app_2280"), Some("performance"));
    }

    #[test]
    fn anything_unlisted_falls_back_to_the_default() {
        assert_eq!(rules().profile_for("zen-browser"), Some("balanced"));
    }

    #[test]
    fn without_a_default_an_unlisted_window_holds_nothing() {
        let rules = Rules::parse("[classes]\n\"foot\" = \"power-saver\"\n").unwrap();
        assert_eq!(rules.profile_for("zen-browser"), None);
        assert_eq!(rules.profile_for("foot"), Some("power-saver"));
    }

    #[test]
    fn a_typo_in_the_rules_is_an_error_rather_than_a_rule_that_never_fires() {
        assert!(Rules::parse("[clases]\n\"foot\" = \"power-saver\"\n").is_err());
    }

    #[test]
    fn the_class_is_read_up_to_the_first_comma() {
        // Titles routinely contain commas; classes never do.
        assert_eq!(
            active_class("activewindow>>foot,vim README.md, line 3"),
            Some("foot")
        );
    }

    #[test]
    fn closing_the_last_window_reports_an_empty_class() {
        assert_eq!(active_class("activewindow>>,"), Some(""));
        assert_eq!(rules().profile_for(""), Some("balanced"));
    }

    #[test]
    fn the_focused_class_is_read_out_of_the_compositors_reply() {
        let reply =
            "Window 55cd -> vim README.md:\n\tmapped: 1\n\tclass: foot\n\tinitialClass: foot\n";
        assert_eq!(class_in_reply(reply), Some("foot"));
    }

    #[test]
    fn no_focused_window_reads_as_nothing_rather_than_an_empty_class() {
        assert_eq!(class_in_reply("Invalid\n"), None);
        assert_eq!(class_in_reply("\tclass: \n"), None);
    }

    #[test]
    fn other_compositor_events_are_ignored() {
        assert_eq!(active_class("workspace>>2"), None);
        // The v2 event carries an address, not a class, and must not be read
        // as one.
        assert_eq!(active_class("activewindowv2>>55cdbe1740c0"), None);
    }
}
