//! Rendering. Read-only for now: the daemon that accepts changes lands next.

use omarchy_power_core::types::{Capabilities, FanMode, HwState, PowerLevel};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Gauge, Paragraph};

/// Temperatures at which the gauge changes colour. Advisory only — nothing acts
/// on these yet.
const WARM_C: u8 = 75;
const HOT_C: u8 = 88;

pub struct Screen<'a> {
    pub backend: &'a str,
    pub model: Option<&'a str>,
    pub state: &'a HwState,
    pub capabilities: Capabilities,
    /// The outcome of the last action, or the last read failure. A failed read
    /// leaves the previous snapshot on screen rather than blanking it.
    pub status: Option<&'a Status>,
    /// True when no daemon is available and nothing can be changed.
    pub read_only: bool,
    /// Units that rewrite the charge threshold at boot. Shown next to the
    /// charge limit, because the setting looks lost rather than overridden.
    pub charge_conflicts: &'a [String],
}

/// A transient line of feedback shown in the footer.
pub struct Status {
    pub text: String,
    pub failed: bool,
}

impl Status {
    pub fn ok(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            failed: false,
        }
    }

    pub fn failed(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            failed: true,
        }
    }
}

pub fn draw(frame: &mut Frame, screen: &Screen) {
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    frame.render_widget(header_line(screen), header);

    let [left, right] =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(body);

    frame.render_widget(
        state_panel(screen.state, screen.capabilities, screen.charge_conflicts),
        left,
    );
    render_sensors(frame, right, screen.state);
    frame.render_widget(footer_line(screen), footer);
}

fn header_line<'a>(screen: &Screen<'a>) -> Paragraph<'a> {
    let mut spans = vec![
        Span::styled(" omarchy-power ", Style::new().bold().reversed()),
        Span::raw(" "),
        Span::styled(screen.backend.to_owned(), Style::new().cyan()),
    ];
    if let Some(model) = screen.model {
        spans.push(Span::styled(format!("  {model}"), Style::new().dark_gray()));
    }
    Paragraph::new(Line::from(spans))
}

fn state_panel<'a>(
    state: &'a HwState,
    caps: Capabilities,
    charge_conflicts: &[String],
) -> Paragraph<'a> {
    // Each row carries the key that changes it, so the panel doubles as the legend.
    let rows = [
        (
            "p",
            "Power level",
            power_level_text(state.power_level),
            caps.power_level,
        ),
        (
            "f",
            "Fan mode",
            fan_mode_text(state.fan_mode),
            caps.fan_mode,
        ),
        (
            "b",
            "Cooler boost",
            switch_text(state.cooler_boost),
            caps.cooler_boost,
        ),
        (
            "s",
            "Battery saver",
            switch_text(state.battery_saver),
            caps.battery_saver,
        ),
        (
            "-/+",
            "Charge limit",
            charge_text(state),
            caps.charge_threshold,
        ),
        ("", "Battery", battery_text(state), true),
    ];

    let mut lines: Vec<Line> = rows
        .into_iter()
        .map(|(key, label, value, supported)| {
            // Unsupported rows stay visible but muted: knowing that the laptop
            // cannot do something is worth a line of screen.
            let value_style = if supported {
                Style::new().bold()
            } else {
                Style::new().dark_gray()
            };
            Line::from(vec![
                Span::styled(format!("{key:>3}  "), Style::new().yellow()),
                Span::styled(format!("{label:<15}"), Style::new().dark_gray()),
                Span::styled(value, value_style),
            ])
        })
        .collect();

    // Right under the charge limit it would push the battery line around as the
    // warning appears and goes; at the bottom the panel keeps its shape.
    if !charge_conflicts.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!(
                "!    {} also sets the charge limit",
                charge_conflicts.join(", ")
            ),
            Style::new().yellow(),
        )));
    }

    Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" state ")
            .padding(ratatui::widgets::Padding::uniform(1)),
    )
}

fn render_sensors(frame: &mut Frame, area: Rect, state: &HwState) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" sensors ")
        .padding(ratatui::widgets::Padding::uniform(1));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // One row per gauge with a blank row between: a two-row gauge puts its
    // label halfway up the bar, which reads badly.
    let [cpu, _, gpu, _, fans] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .areas(inner);

    render_temp(
        frame,
        cpu,
        "CPU",
        state.sensors.cpu_temp_c,
        state.sensors.cpu_fan_percent,
    );
    render_temp(
        frame,
        gpu,
        "GPU",
        state.sensors.gpu_temp_c,
        state.sensors.gpu_fan_percent,
    );

    let rpm: Vec<String> = state
        .sensors
        .fan_rpm
        .iter()
        .enumerate()
        .map(|(i, rpm)| format!("fan{}: {rpm} rpm", i + 1))
        .collect();
    let text = if rpm.is_empty() {
        "no tachometer readings".to_owned()
    } else {
        rpm.join("  ")
    };
    frame.render_widget(
        Paragraph::new(Span::styled(text, Style::new().dark_gray()))
            .wrap(ratatui::widgets::Wrap { trim: true }),
        fans,
    );
}

fn render_temp(frame: &mut Frame, area: Rect, label: &str, temp: Option<u8>, fan: Option<u8>) {
    let Some(temp) = temp else {
        frame.render_widget(
            Paragraph::new(format!("{label}  unavailable")).dark_gray(),
            area,
        );
        return;
    };

    let colour = match temp {
        t if t >= HOT_C => Color::Red,
        t if t >= WARM_C => Color::Yellow,
        _ => Color::Green,
    };
    let fan = fan.map(|f| format!("  fan {f}%")).unwrap_or_default();

    // The reading sits beside the bar rather than on top of it: a centred gauge
    // label is legible in a terminal but unreadable in a plain-text copy.
    let [text, bar] = Layout::horizontal([Constraint::Length(19), Constraint::Min(4)]).areas(area);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(format!("{label}  "), Style::new().dark_gray()),
            Span::styled(format!("{temp}°C"), Style::new().fg(colour).bold()),
            Span::styled(fan, Style::new().dark_gray()),
        ])),
        text,
    );
    frame.render_widget(
        Gauge::default()
            .gauge_style(Style::new().fg(colour))
            // Percent of a 100 degree scale: close enough to a thermal budget
            // to read at a glance, and it never needs a per-model calibration.
            .percent(u16::from(temp).min(100))
            .label(""),
        bar,
    );
}

fn power_level_text(level: Option<PowerLevel>) -> String {
    match level {
        Some(PowerLevel::Performance) => "performance",
        Some(PowerLevel::Balanced) => "balanced",
        Some(PowerLevel::PowerSaver) => "power saver",
        None => "unknown",
    }
    .to_owned()
}

fn fan_mode_text(mode: Option<FanMode>) -> String {
    match mode {
        Some(FanMode::Auto) => "auto",
        Some(FanMode::Silent) => "silent",
        Some(FanMode::Aggressive) => "aggressive",
        None => "unknown",
    }
    .to_owned()
}

fn switch_text(value: Option<bool>) -> String {
    match value {
        Some(true) => "on",
        Some(false) => "off",
        None => "unsupported",
    }
    .to_owned()
}

fn charge_text(state: &HwState) -> String {
    state
        .battery
        .charge_end_threshold
        .map_or_else(|| "unsupported".to_owned(), |t| format!("{t}%"))
}

fn battery_text(state: &HwState) -> String {
    let capacity = state
        .battery
        .capacity_percent
        .map_or_else(|| "?".to_owned(), |c| format!("{c}%"));
    match state.battery.on_ac {
        Some(true) => format!("{capacity} (on AC)"),
        Some(false) => format!("{capacity} (on battery)"),
        None => capacity,
    }
}

fn footer_line<'a>(screen: &Screen<'a>) -> Paragraph<'a> {
    if let Some(status) = screen.status {
        let style = if status.failed {
            Style::new().red().bold()
        } else {
            Style::new().green()
        };
        return Paragraph::new(Span::styled(format!(" {}", status.text), style));
    }

    let hint = if screen.read_only {
        " q quit   r refresh   read-only: omarchy-powerd is not running"
    } else {
        " q quit   r refresh   p/f cycle   b/s toggle   -/+ charge limit"
    };
    Paragraph::new(Span::styled(hint, Style::new().dark_gray()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use omarchy_power_core::types::{Battery, Sensors};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn sample_state() -> HwState {
        HwState {
            power_level: Some(PowerLevel::Balanced),
            fan_mode: Some(FanMode::Auto),
            cooler_boost: Some(false),
            battery_saver: Some(false),
            sensors: Sensors {
                cpu_temp_c: Some(66),
                gpu_temp_c: Some(51),
                cpu_fan_percent: Some(70),
                gpu_fan_percent: Some(40),
                fan_rpm: vec![3555, 3555, 0, 0],
            },
            battery: Battery {
                capacity_percent: Some(99),
                charge_end_threshold: Some(80),
                on_ac: Some(true),
            },
        }
    }

    fn all_caps() -> Capabilities {
        Capabilities {
            power_level: true,
            fan_mode: true,
            cooler_boost: true,
            battery_saver: true,
            charge_threshold: true,
        }
    }

    struct Case<'a> {
        state: &'a HwState,
        caps: Capabilities,
        status: Option<&'a Status>,
        read_only: bool,
        charge_conflicts: Vec<String>,
    }

    impl<'a> Case<'a> {
        fn new(state: &'a HwState) -> Self {
            Self {
                state,
                caps: all_caps(),
                status: None,
                read_only: false,
                charge_conflicts: Vec::new(),
            }
        }
    }

    fn render(case: Case<'_>) -> String {
        render_sized(case, 100, 24)
    }

    fn render_sized(case: Case<'_>, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| {
                draw(
                    frame,
                    &Screen {
                        backend: "msi-ec",
                        model: Some("1587EMS1.106"),
                        state: case.state,
                        capabilities: case.caps,
                        status: case.status,
                        read_only: case.read_only,
                        charge_conflicts: &case.charge_conflicts,
                    },
                )
            })
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    #[test]
    fn shows_hardware_state_and_sensors() {
        let state = sample_state();
        let screen = render(Case::new(&state));

        assert!(screen.contains("msi-ec"), "backend name missing");
        assert!(screen.contains("1587EMS1.106"), "firmware version missing");
        assert!(screen.contains("balanced"), "power level missing");
        assert!(screen.contains("66"), "cpu temperature missing");
        assert!(screen.contains("3555 rpm"), "fan speed missing");
        assert!(screen.contains("99% (on AC)"), "battery state missing");
        assert!(screen.contains("80%"), "charge limit missing");
    }

    #[test]
    fn a_failure_takes_over_the_footer_but_keeps_the_last_snapshot() {
        let state = sample_state();
        let status = Status::failed("reading shift_mode: EIO");
        let screen = render(Case {
            status: Some(&status),
            ..Case::new(&state)
        });

        assert!(screen.contains("EIO"), "error not shown");
        assert!(screen.contains("balanced"), "last known state was dropped");
    }

    #[test]
    fn unsupported_hardware_reads_as_such_rather_than_as_zero() {
        let state = HwState::default();
        let screen = render(Case {
            caps: Capabilities::default(),
            ..Case::new(&state)
        });

        assert!(
            screen.contains("unsupported"),
            "missing capabilities unclear"
        );
        assert!(screen.contains("unavailable"), "missing sensors unclear");
        assert!(
            !screen.contains("0°C"),
            "absent temperature rendered as a reading"
        );
    }

    #[test]
    fn the_footer_explains_why_keys_do_nothing_without_a_daemon() {
        let state = sample_state();
        let live = render(Case::new(&state));
        assert!(live.contains("p/f cycle"), "controls not advertised");

        let read_only = render(Case {
            read_only: true,
            ..Case::new(&state)
        });
        assert!(
            read_only.contains("omarchy-powerd is not running"),
            "read-only mode unexplained"
        );
        assert!(
            !read_only.contains("p/f cycle"),
            "offers keys that do nothing"
        );
    }

    #[test]
    fn a_unit_that_overwrites_the_charge_limit_is_named_on_screen() {
        let state = sample_state();

        let quiet = render(Case::new(&state));
        assert!(
            !quiet.contains("also sets the charge limit"),
            "warns without anything to warn about"
        );

        let conflicted = render(Case {
            charge_conflicts: vec!["battery-charge-threshold.service".to_owned()],
            ..Case::new(&state)
        });
        assert!(
            conflicted.contains("battery-charge-threshold.service"),
            "the unit doing the overwriting is what the user has to go and disable"
        );
    }

    /// A terminal too small to hold the layout must not panic.
    #[test]
    fn survives_a_tiny_terminal() {
        let state = sample_state();
        render_sized(Case::new(&state), 20, 3);
        render_sized(Case::new(&state), 1, 1);
    }
}
