//! Rendering. Read-only for now: the daemon that accepts changes lands next.

use omarchy_power_core::types::{FanMode, HwState, PowerLevel};
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
    /// Set when reading the last snapshot failed; the previous one stays on
    /// screen so a transient error does not blank the display.
    pub error: Option<&'a str>,
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

    frame.render_widget(state_panel(screen.state), left);
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

fn state_panel(state: &HwState) -> Paragraph<'_> {
    let rows = [
        ("Power level", power_level_text(state.power_level)),
        ("Fan mode", fan_mode_text(state.fan_mode)),
        ("Cooler boost", switch_text(state.cooler_boost)),
        ("Battery saver", switch_text(state.battery_saver)),
        ("Charge limit", charge_text(state)),
        ("Battery", battery_text(state)),
    ];

    let lines: Vec<Line> = rows
        .into_iter()
        .map(|(label, value)| {
            Line::from(vec![
                Span::styled(format!("{label:<15}"), Style::new().dark_gray()),
                Span::styled(value, Style::new().bold()),
            ])
        })
        .collect();

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
    let fan = fan.map(|f| format!("   fan {f}%")).unwrap_or_default();

    frame.render_widget(
        Gauge::default()
            .gauge_style(Style::new().fg(colour))
            // Percent of a 100 degree scale: close enough to a thermal budget
            // to read at a glance, and it never needs a per-model calibration.
            .percent(u16::from(temp).min(100))
            .label(format!("{label}  {temp}°C{fan}")),
        area,
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
    match screen.error {
        Some(error) => Paragraph::new(Span::styled(format!(" {error}"), Style::new().red().bold())),
        None => Paragraph::new(Span::styled(
            " q quit   r refresh   (read-only until the daemon lands)",
            Style::new().dark_gray(),
        )),
    }
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

    fn render(state: &HwState, error: Option<&str>, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| {
                draw(
                    frame,
                    &Screen {
                        backend: "msi-ec",
                        model: Some("1587EMS1.106"),
                        state,
                        error,
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
        let screen = render(&sample_state(), None, 100, 24);

        assert!(screen.contains("msi-ec"), "backend name missing");
        assert!(screen.contains("1587EMS1.106"), "firmware version missing");
        assert!(screen.contains("balanced"), "power level missing");
        assert!(screen.contains("66"), "cpu temperature missing");
        assert!(screen.contains("3555 rpm"), "fan speed missing");
        assert!(screen.contains("99% (on AC)"), "battery state missing");
        assert!(screen.contains("80%"), "charge limit missing");
    }

    #[test]
    fn an_error_takes_over_the_footer_but_keeps_the_last_snapshot() {
        let screen = render(&sample_state(), Some("reading shift_mode: EIO"), 100, 24);

        assert!(screen.contains("EIO"), "error not shown");
        assert!(screen.contains("balanced"), "last known state was dropped");
    }

    #[test]
    fn unsupported_hardware_reads_as_such_rather_than_as_zero() {
        let screen = render(&HwState::default(), None, 100, 24);

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

    /// A terminal too small to hold the layout must not panic.
    #[test]
    fn survives_a_tiny_terminal() {
        render(&sample_state(), None, 20, 3);
        render(&sample_state(), None, 1, 1);
    }
}
