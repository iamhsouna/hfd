use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::widgets::Widget;

use crate::util;

const TRACK: Color = Color::Indexed(238);
const GREEN: Color = Color::Green;
const YELLOW: Color = Color::Yellow;
const RED: Color = Color::Red;

/// A radial speedometer, drawn with a colored arc and a needle.
pub struct Speedometer<'a> {
    pub speed: f64,
    /// Recent speed samples (bytes/sec), used to scale the dial.
    pub history: &'a [u64],
    pub average: f64,
    pub paused: bool,
}

impl Widget for Speedometer<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width < 12 || area.height < 5 {
            return;
        }

        let cx = area.x as f64 + area.width as f64 / 2.0;
        let cy = area.y as f64 + area.height as f64 - 2.0;
        let rx = (area.width as f64 / 2.0 - 2.0).max(4.0);
        // Terminal cells are roughly twice as tall as they are wide.
        let ry = (rx / 2.0).min(area.height as f64 - 3.0).max(2.0);

        let peak = self
            .history
            .iter()
            .copied()
            .max()
            .unwrap_or(0)
            .max(self.speed as u64)
            .max(1);
        let max = peak as f64;
        let fraction = (self.speed / max).clamp(0.0, 1.0);

        // Arc: one point per screen column guarantees a continuous curve.
        let mut column = -rx.ceil() as i64;
        while column <= rx.ceil() as i64 {
            let ratio = column as f64 / rx;
            if ratio.abs() <= 1.0 {
                let y = cy - ry * (1.0 - ratio * ratio).sqrt();
                let t = arc_fraction(ratio);
                let color = if t <= fraction { zone_color(t) } else { TRACK };
                let cell = if t <= fraction { '•' } else { '·' };
                plot(
                    buf,
                    area,
                    cx + column as f64,
                    y,
                    cell,
                    Style::new().fg(color),
                );
            }
            column += 1;
        }

        // Major ticks.
        for step in 0..=8 {
            let t = step as f64 / 8.0;
            let angle = std::f64::consts::PI + t * std::f64::consts::PI;
            for depth in [0.90, 0.96, 1.0] {
                let x = cx + rx * depth * angle.cos();
                let y = cy + ry * depth * angle.sin();
                plot(buf, area, x, y, '│', Style::new().fg(Color::Gray));
            }
        }

        // Needle.
        let angle = std::f64::consts::PI + fraction * std::f64::consts::PI;
        let needle_len = rx * 0.86;
        let steps = (needle_len * 2.0).max(8.0) as i64;
        for step in 0..=steps {
            let r = step as f64 / steps as f64;
            let x = cx + needle_len * r * angle.cos();
            let y = cy + ry * 0.95 * r * angle.sin();
            plot(buf, area, x, y, '●', Style::new().fg(Color::White).bold());
        }
        plot(buf, area, cx, cy, '●', Style::new().fg(Color::Red).bold());

        // Digital readout.
        let speed_text = util::format_speed(self.speed);
        let readout_x = cx - speed_text.chars().count() as f64 / 2.0;
        let readout_y = (cy - ry * 0.45).round();
        buf.set_string(
            readout_x.round().max(area.x as f64) as u16,
            readout_y.max(area.y as f64) as u16,
            &speed_text,
            Style::new().fg(Color::White).bold(),
        );

        let label = if self.paused {
            "PAUSED".to_string()
        } else {
            format!("avg {}", util::format_speed(self.average))
        };
        let label_x = cx - label.chars().count() as f64 / 2.0;
        let label_y = (cy - ry * 0.45 + 1.0).round();
        buf.set_string(
            label_x.round().max(area.x as f64) as u16,
            label_y.max(area.y as f64) as u16,
            &label,
            Style::new().fg(if self.paused { YELLOW } else { Color::DarkGray }),
        );

        // Scale endpoints.
        buf.set_string(area.x, area.y, "0", Style::new().fg(Color::DarkGray));
        let top = util::format_speed(max);
        let top_x = (area.x as f64 + area.width as f64 - top.chars().count() as f64 - 1.0).max(0.0);
        buf.set_string(top_x as u16, area.y, &top, Style::new().fg(Color::DarkGray));
    }
}

fn arc_fraction(ratio: f64) -> f64 {
    // ratio = cos(angle), sweep 180°..360° => t goes 0..1 left to right.
    1.0 - ratio.clamp(-1.0, 1.0).acos() / std::f64::consts::PI
}

fn zone_color(t: f64) -> Color {
    if t <= 0.6 {
        GREEN
    } else if t <= 0.85 {
        YELLOW
    } else {
        RED
    }
}

fn plot(buf: &mut Buffer, area: Rect, x: f64, y: f64, ch: char, style: Style) {
    let xi = x.round();
    let yi = y.round();
    if xi < area.x as f64
        || yi < area.y as f64
        || xi >= (area.x + area.width) as f64
        || yi >= (area.y + area.height) as f64
    {
        return;
    }
    buf.set_string(xi as u16, yi as u16, ch.to_string(), style);
}
