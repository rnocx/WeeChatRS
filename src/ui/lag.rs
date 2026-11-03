//! The lag readout in the toolbar, for the network of the buffer on screen.

use crate::relay::backend::Lag;
use egui::{Color32, FontId, Sense, Stroke, StrokeKind, Vec2};
use std::time::{Duration, Instant};

/// Lag from here on is orange…
const SLOW: Duration = Duration::from_secs(1);
/// …and from here on red. An unanswered check this old reads "no reply".
const VERY_SLOW: Duration = Duration::from_secs(5);

/// What the checks on one network (or one connection) have found so far.
#[derive(Debug, Default, Clone, Copy)]
pub struct NetworkLag {
    /// The last answer: `Measured` or `Low`.
    last: Option<Lag>,
    /// A check is out, sent at this moment.
    waiting_since: Option<Instant>,
    /// When the last answer came.
    checked_at: Option<Instant>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Level {
    Unknown,
    Good,
    Slow,
    Bad,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Reading {
    pub text: String,
    pub level: Level,
    /// A check is out: the text changes by itself as time passes.
    pub ticking: bool,
}

impl NetworkLag {
    pub fn apply(&mut self, lag: Lag) {
        match lag {
            // WeeChat repeats a waiting check every second, rounded: keep the first.
            Lag::Waiting(sent) => { self.waiting_since.get_or_insert(sent); }
            answer => {
                self.last = Some(answer);
                self.waiting_since = None;
                self.checked_at = Some(Instant::now());
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.last.is_none() && self.waiting_since.is_none()
    }

    pub fn reading(&self, now: Instant) -> Reading {
        let waited = self.waiting_since.map(|t| now.saturating_duration_since(t));
        if let Some(w) = waited.filter(|w| *w >= VERY_SLOW) {
            return Reading { text: format!("no reply {} s", w.as_secs()), level: Level::Bad, ticking: true };
        }
        let ticking = waited.is_some();
        let measured = match self.last {
            Some(Lag::Measured(d)) => d,
            Some(_) if waited.is_none_or(|w| w < SLOW) => {
                return Reading { text: "ok".into(), level: Level::Good, ticking };
            }
            Some(_) => Duration::ZERO,
            None if waited.is_none_or(|w| w < SLOW) => {
                return Reading { text: "…".into(), level: Level::Unknown, ticking };
            }
            None => Duration::ZERO,
        };
        // A check out for longer than the last answer took is lag already, as in WeeChat.
        let lag = waited.map_or(measured, |w| w.max(measured));
        Reading { text: format_lag(lag), level: level_of(lag), ticking }
    }

    /// "Checked 14 s ago", or what's going on instead.
    pub fn checked_text(&self, now: Instant) -> String {
        match (self.waiting_since, self.checked_at) {
            (Some(t), _) if now.saturating_duration_since(t) >= SLOW => {
                format!("Waiting {} s for an answer", now.saturating_duration_since(t).as_secs())
            }
            (_, Some(t)) => format!("Checked {} s ago", now.saturating_duration_since(t).as_secs()),
            _ => "Not measured yet".into(),
        }
    }
}

fn format_lag(lag: Duration) -> String {
    if lag < SLOW {
        format!("{} ms", lag.as_millis())
    } else {
        format!("{:.1} s", lag.as_secs_f64())
    }
}

fn level_of(lag: Duration) -> Level {
    if lag < SLOW { Level::Good } else if lag < VERY_SLOW { Level::Slow } else { Level::Bad }
}

/// The lag WeeChat keeps in an IRC server buffer's `lag` local variable: "0.734" once
/// answered (or while waiting, under a second), whole seconds while a check is still
/// waiting, and nothing under irc.network.lag_min_show.
pub fn weechat_lag(vars: &serde_json::Map<String, serde_json::Value>, now: Instant) -> Lag {
    let Some(text) = vars.get("lag").and_then(|v| v.as_str()) else { return Lag::Low };
    let Ok(secs) = text.trim().parse::<f64>() else { return Lag::Low };
    let secs = Duration::from_secs_f64(secs.max(0.0));
    if text.contains('.') {
        Lag::Measured(secs)
    } else {
        Lag::Waiting(now.checked_sub(secs).unwrap_or(now))
    }
}

/// A buffer's local variables without `lag`, to tell a lag update, which changes
/// nothing else, from a change the buffer list has to be fetched again for.
pub fn vars_without_lag(vars: &serde_json::Map<String, serde_json::Value>) -> String {
    let mut vars = vars.clone();
    vars.remove("lag");
    serde_json::Value::Object(vars).to_string()
}

fn level_color(level: Level, ui: &egui::Ui) -> Color32 {
    match level {
        Level::Good => Color32::from_rgb(50, 205, 50),
        Level::Slow => Color32::from_rgb(240, 162, 74),
        Level::Bad => Color32::from_rgb(255, 110, 100),
        Level::Unknown => ui.visuals().weak_text_color(),
    }
}

/// The readout: signal bars, the network's name and the lag, in a pill that fills
/// in orange or red when the lag is high.
pub fn draw(ui: &mut egui::Ui, network: &str, reading: &Reading) -> egui::Response {
    let color = level_color(reading.level, ui);
    let name_color = ui.visuals().text_color();
    let name = ui.painter().layout_no_wrap(network.to_string(), FontId::proportional(12.0), name_color);
    let value = ui.painter().layout_no_wrap(reading.text.clone(), FontId::monospace(12.0), color);
    let (pad, bars_w, gap) = (8.0, 12.0, 5.0);
    let size = Vec2::new(pad + bars_w + gap + name.size().x + gap + value.size().x + pad, 22.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let painter = ui.painter();
    let (fill, border) = match reading.level {
        Level::Slow | Level::Bad => (color.gamma_multiply(0.12), color.gamma_multiply(0.45)),
        _ => (Color32::TRANSPARENT, ui.visuals().widgets.noninteractive.bg_stroke.color),
    };
    painter.rect(rect, 11.0, fill, Stroke::new(1.0, border), StrokeKind::Inside);

    let lit = match reading.level {
        Level::Good => 4,
        Level::Slow => 3,
        Level::Bad if reading.text.starts_with("no reply") => 1,
        Level::Bad => 2,
        Level::Unknown => 0,
    };
    let dim = ui.visuals().weak_text_color().gamma_multiply(0.5);
    let base = rect.center().y + 5.0;
    for i in 0..4 {
        let x = rect.left() + pad + 1.0 + i as f32 * 3.4;
        let top = base - 3.0 - i as f32 * 2.4;
        let c = if i < lit { color } else { dim };
        painter.line_segment([egui::pos2(x, base), egui::pos2(x, top)], Stroke::new(1.8, c));
    }

    let mut x = rect.left() + pad + bars_w + gap;
    painter.galley(egui::pos2(x, rect.center().y - name.size().y / 2.0), name.clone(), name_color);
    x += name.size().x + gap;
    painter.galley(egui::pos2(x, rect.center().y - value.size().y / 2.0), value, color);
    response
}

impl crate::ui::app::WeeChatApp {
    /// The lag to show for the buffer on screen: the name it goes under, its checks,
    /// and how it's measured. None where there's no network to measure.
    pub(crate) fn current_lag(&self) -> Option<(String, NetworkLag, &'static str)> {
        use crate::ui::app::BackendType;
        let buffer = self.buffer_by_id(self.selected_buffer_id.as_deref()?)?;
        if buffer.kind == crate::ui::channel_list::KIND {
            return None;
        }
        let prefix = buffer.id.split_once('/')?.0;
        let conn = self.connections.iter().find(|c| c.prefix == prefix)?;
        if !conn.client.is_connected() {
            return None;
        }
        if let Some(lag) = conn.lag.get(&buffer.server).filter(|l| !l.is_empty()) {
            let name = self.network_server_buffer(&buffer.id).map_or_else(|| buffer.server.clone(), |b| b.name);
            let how = match conn.backend_type {
                BackendType::WeeChat => "Measured by WeeChat (irc.network.lag_check). It reports nothing under irc.network.lag_min_show, 500 ms by default.",
                _ => "A PING timed to its PONG. Through soju or ZNC that's the hop to the bouncer, not on to the IRC server.",
            };
            return Some((name, *lag, how));
        }
        if conn.link_lag.is_empty() {
            return None;
        }
        let how = match conn.backend_type {
            BackendType::Injected => "Round trip to Injected's server. The IRC networks behind it aren't measured.",
            _ => "Round trip to the WeeChat relay.",
        };
        Some((conn.label.clone(), conn.link_lag, how))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(lag: &NetworkLag, now: Instant) -> (String, Level) {
        let r = lag.reading(now);
        (r.text, r.level)
    }

    #[test]
    fn readings_follow_the_checks() {
        let t0 = Instant::now();
        let mut lag = NetworkLag::default();
        assert!(lag.is_empty());
        assert_eq!(at(&lag, t0), ("…".into(), Level::Unknown));

        lag.apply(Lag::Waiting(t0));
        assert_eq!(at(&lag, t0 + Duration::from_millis(300)), ("…".into(), Level::Unknown));
        lag.apply(Lag::Measured(Duration::from_millis(48)));
        assert_eq!(at(&lag, t0 + Duration::from_secs(1)), ("48 ms".into(), Level::Good));

        // A check out for longer than the last answer took counts up, like WeeChat's.
        let t1 = t0 + Duration::from_secs(30);
        lag.apply(Lag::Waiting(t1));
        assert_eq!(at(&lag, t1 + Duration::from_millis(20)), ("48 ms".into(), Level::Good));
        assert_eq!(at(&lag, t1 + Duration::from_millis(1800)), ("1.8 s".into(), Level::Slow));
        assert_eq!(at(&lag, t1 + Duration::from_secs(34)), ("no reply 34 s".into(), Level::Bad));
        // A repeated waiting report doesn't restart the count.
        lag.apply(Lag::Waiting(t1 + Duration::from_secs(33)));
        assert_eq!(at(&lag, t1 + Duration::from_secs(34)), ("no reply 34 s".into(), Level::Bad));

        lag.apply(Lag::Measured(Duration::from_millis(7200)));
        assert_eq!(at(&lag, t1 + Duration::from_secs(40)), ("7.2 s".into(), Level::Bad));
        lag.apply(Lag::Low);
        assert_eq!(at(&lag, t1 + Duration::from_secs(41)), ("ok".into(), Level::Good));
    }

    #[test]
    fn weechat_lag_variable() {
        let now = Instant::now();
        let vars = |lag: Option<&str>| {
            let mut m = serde_json::Map::new();
            m.insert("type".into(), "server".into());
            if let Some(l) = lag { m.insert("lag".into(), l.into()); }
            m
        };
        assert_eq!(weechat_lag(&vars(None), now), Lag::Low);
        assert_eq!(weechat_lag(&vars(Some("0.734")), now), Lag::Measured(Duration::from_millis(734)));
        assert_eq!(weechat_lag(&vars(Some("12")), now), Lag::Waiting(now - Duration::from_secs(12)));
        assert_eq!(vars_without_lag(&vars(Some("3"))), vars_without_lag(&vars(None)));
    }
}
