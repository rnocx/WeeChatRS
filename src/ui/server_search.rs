//! Search of the server's history (soju.im/search): the results view that takes the
//! chat's place, and the conversation around a result (CHATHISTORY AROUND).

use egui::{Color32, FontId, Frame, Label, Margin, ScrollArea};
use egui::text::LayoutJob;

use crate::relay::models::Line;
use crate::ui::ansi::ANSIParser;
use crate::ui::app::WeeChatApp;
use crate::ui::theme::AppTheme;

pub(crate) struct ServerSearch {
    pub(crate) query: String,
    /// What was searched, for the header: a buffer name or "all buffers".
    pub(crate) scope: String,
    pub(crate) request: String,
    /// Buffers searched, and those that haven't answered yet.
    pub(crate) searched: usize,
    pub(crate) waiting: usize,
    /// Matches as (full buffer id, line), newest first.
    pub(crate) hits: Vec<(String, Line)>,
    pub(crate) errors: Vec<String>,
    pub(crate) open: Option<OpenHit>,
}

/// A result whose surrounding conversation is shown below it.
pub(crate) struct OpenHit {
    index: usize,
    request: String,
    lines: Option<Vec<Line>>,
    error: Option<String>,
}

/// What the view asks the app to do once it's drawn.
enum Action {
    Close,
    Toggle(usize),
    OpenBuffer(String),
}

const ALL_BUFFERS: &str = "all buffers";

impl WeeChatApp {
    /// Name of the network (server buffer) heading `buffer_id`'s group.
    fn network_name(&self, buffer_id: &str) -> Option<String> {
        let buffer = self.buffer_by_id(buffer_id)?;
        let conn = buffer_id.split('/').next()?;
        self.buffers.iter()
            .find(|b| b.kind == "server" && b.server == buffer.server && b.id.split('/').next() == Some(conn))
            .map(|b| b.name.clone())
    }

    fn next_request(&mut self, kind: &str) -> String {
        self.next_request_id += 1;
        format!("{}{}", kind, self.next_request_id)
    }

    /// Whether the selected buffer's connection can search its server.
    pub(crate) fn can_search_server(&self) -> bool {
        self.selected_buffer_id.as_deref()
            .and_then(|id| self.client_for_buffer(id))
            .is_some_and(|(client, _)| client.supports_search())
    }

    /// Search the server for the search bar's text: in the selected buffer, or in
    /// every channel and private conversation on its connection.
    pub(crate) fn start_server_search(&mut self, all_buffers: bool) {
        let query = self.search_text.trim().to_string();
        let Some(selected) = self.selected_buffer_id.clone() else { return };
        if query.is_empty() { return; }
        let Some(conn_prefix) = selected.split('/').next().map(|p| format!("{}/", p)) else { return };
        // A server buffer has no history of its own: search everything on the connection.
        let all_buffers = all_buffers || !self.buffer_by_id(&selected)
            .is_some_and(|b| matches!(b.kind.as_str(), "channel" | "private"));
        let targets: Vec<String> = if all_buffers {
            self.buffers.iter()
                .filter(|b| b.id.starts_with(&conn_prefix) && matches!(b.kind.as_str(), "channel" | "private"))
                .map(|b| b.id.clone())
                .collect()
        } else {
            self.buffer_by_id(&selected)
                .filter(|b| matches!(b.kind.as_str(), "channel" | "private"))
                .map(|b| vec![b.id.clone()])
                .unwrap_or_default()
        };
        let scope = if all_buffers {
            ALL_BUFFERS.to_string()
        } else {
            self.buffer_by_id(&selected).map(|b| b.name.clone()).unwrap_or_default()
        };
        let request = self.next_request("search");
        let mut sent = 0;
        for id in &targets {
            if let Some((client, raw)) = self.client_for_buffer(id) {
                client.search(&raw, &query, &request);
                sent += 1;
            }
        }
        self.server_search = Some(ServerSearch {
            query,
            scope,
            request,
            searched: sent,
            waiting: sent,
            hits: Vec::new(),
            errors: if sent == 0 { vec!["Only channels and private conversations can be searched".to_string()] } else { Vec::new() },
            open: None,
        });
    }

    /// A `HistoryResults` from connection `conn_prefix`.
    pub(crate) fn on_history_results(&mut self, conn_prefix: &str, request: &str, lines: Vec<(String, Line)>, error: Option<String>) {
        let Some(search) = self.server_search.as_mut() else { return };
        let lines = lines.into_iter().map(|(id, line)| (format!("{}/{}", conn_prefix, id), line));
        if search.request == request {
            search.waiting = search.waiting.saturating_sub(1);
            search.hits.extend(lines);
            search.hits.sort_by(|a, b| b.1.timestamp.cmp(&a.1.timestamp));
            if let Some(e) = error.filter(|e| !search.errors.contains(e)) {
                search.errors.push(e);
            }
        } else if let Some(open) = search.open.as_mut().filter(|o| o.request == request) {
            let buffer = search.hits.get(open.index).map(|h| h.0.clone()).unwrap_or_default();
            let mut context: Vec<Line> = lines.filter(|(id, _)| *id == buffer).map(|(_, l)| l).collect();
            context.sort_by_key(|l| l.timestamp);
            open.lines = Some(context);
            open.error = error;
        }
    }

    fn toggle_hit(&mut self, index: usize) {
        let Some(search) = self.server_search.as_ref() else { return };
        if search.open.as_ref().is_some_and(|o| o.index == index) {
            self.server_search.as_mut().unwrap().open = None;
            return;
        }
        let Some((buffer_id, line)) = search.hits.get(index).cloned() else { return };
        let request = self.next_request("context");
        let timestamp = line.timestamp.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
        if let Some((client, raw)) = self.client_for_buffer(&buffer_id) {
            client.fetch_context(&raw, &timestamp, &request);
        }
        self.server_search.as_mut().unwrap().open = Some(OpenHit { index, request, lines: None, error: None });
    }

    /// The results, in place of the chat.
    pub(crate) fn draw_server_search(&mut self, ui: &mut egui::Ui, theme: &AppTheme, accent: Color32, muted: Color32) {
        let Some(search) = self.server_search.as_ref() else { return };
        let font = FontId::new(self.font_size, crate::ui::fonts::chat_family());
        let mut action = None;

        Frame::NONE.inner_margin(Margin::symmetric(16, 10)).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(format!("“{}” in {}", search.query, search.scope)).strong());
                if search.waiting > 0 {
                    ui.spinner();
                    ui.label(egui::RichText::new(format!("Searching {} of {}…", search.searched - search.waiting + 1, search.searched)).color(muted).small());
                } else {
                    let n = search.hits.len();
                    ui.label(egui::RichText::new(match n { 0 => "No matches".to_string(), 1 => "1 match".to_string(), _ => format!("{} matches", n) }).color(muted).small());
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Back to chat").clicked() { action = Some(Action::Close); }
                });
            });
            for e in &search.errors {
                ui.label(egui::RichText::new(e).color(Color32::from_rgb(220, 90, 90)).small());
            }
        });
        ui.separator();

        ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            Frame::NONE.inner_margin(Margin::symmetric(16, 8)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                for (i, (buffer_id, line)) in search.hits.iter().enumerate() {
                    let buffer_name = self.buffer_by_id(buffer_id).map(|b| b.name.clone())
                        .unwrap_or_else(|| buffer_id.rsplit('/').next().unwrap_or_default().to_string());
                    // Across buffers the same channel name can come from two networks.
                    let network = (search.scope == ALL_BUFFERS).then(|| self.network_name(buffer_id)).flatten();
                    let is_open = search.open.as_ref().is_some_and(|o| o.index == i);
                    let row = Frame::NONE
                        .fill(if is_open { accent.linear_multiply(0.12) } else { Color32::TRANSPARENT })
                        .inner_margin(Margin::symmetric(6, 4))
                        .corner_radius(4.0)
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal_wrapped(|ui| {
                                ui.label(egui::RichText::new(line.timestamp.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string()).font(font.clone()).color(muted));
                                ui.label(egui::RichText::new(&buffer_name).font(font.clone()).color(accent));
                                if let Some(network) = &network {
                                    ui.label(egui::RichText::new(network).font(font.clone()).color(muted));
                                }
                                ui.add(Label::new(line_job(line, &font, theme, &search.query)).wrap());
                            });
                        });
                    let row = ui.interact(row.response.rect, ui.scope_id().with(("server_search_hit", i)), egui::Sense::click());
                    if row.hovered() { ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
                    if row.clicked() { action = Some(Action::Toggle(i)); }

                    if let Some(open) = search.open.as_ref().filter(|o| o.index == i) {
                        Frame::NONE
                            .stroke(egui::Stroke::new(1.0, accent.linear_multiply(0.4)))
                            .corner_radius(4.0)
                            .inner_margin(Margin::same(8))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                match (&open.lines, &open.error) {
                                    (_, Some(e)) => { ui.label(egui::RichText::new(e).color(muted).small()); }
                                    (None, None) => { ui.spinner(); }
                                    (Some(lines), None) => {
                                        for l in lines {
                                            let is_hit = l.timestamp == line.timestamp && l.message == line.message;
                                            let fill = if is_hit { accent.linear_multiply(0.18) } else { Color32::TRANSPARENT };
                                            Frame::NONE.fill(fill).inner_margin(Margin::symmetric(4, 1)).show(ui, |ui| {
                                                ui.set_width(ui.available_width());
                                                ui.horizontal_wrapped(|ui| {
                                                    ui.label(egui::RichText::new(l.timestamp.with_timezone(&chrono::Local).format("%H:%M").to_string()).font(font.clone()).color(muted));
                                                    ui.add(Label::new(line_job(l, &font, theme, "")).wrap());
                                                });
                                            });
                                        }
                                    }
                                }
                                ui.add_space(4.0);
                                if ui.button(format!("Open {}", buffer_name)).clicked() {
                                    action = Some(Action::OpenBuffer(buffer_id.clone()));
                                }
                            });
                    }
                    ui.add_space(2.0);
                }
            });
        });

        match action {
            Some(Action::Close) => self.server_search = None,
            Some(Action::Toggle(i)) => self.toggle_hit(i),
            Some(Action::OpenBuffer(id)) => {
                self.server_search = None;
                self.show_search = false;
                self.search_text.clear();
                self.select_buffer(id);
            }
            None => {}
        }
    }
}

/// `nick message`, in the chat's colours, with `query` underlined where it occurs.
fn line_job(line: &Line, font: &FontId, theme: &AppTheme, query: &str) -> LayoutJob {
    let mut job = LayoutJob::default();
    for s in ANSIParser::parse(&line.prefix) {
        job.append(&s.text, 0.0, s.style.to_format(font.clone(), theme));
    }
    job.append(" ", 0.0, egui::TextFormat { font_id: font.clone(), ..Default::default() });
    let query = query.to_lowercase();
    for s in ANSIParser::parse(&line.message) {
        let format = s.style.to_format(font.clone(), theme);
        let lower = s.text.to_lowercase();
        // Byte offsets line up only when lowercasing keeps lengths; otherwise skip marking.
        if query.is_empty() || lower.len() != s.text.len() {
            job.append(&s.text, 0.0, format);
            continue;
        }
        let mut pos = 0;
        while let Some(found) = lower[pos..].find(&query) {
            let (start, end) = (pos + found, pos + found + query.len());
            job.append(&s.text[pos..start], 0.0, format.clone());
            let mut marked = format.clone();
            marked.underline = egui::Stroke::new(1.5, Color32::from_rgb(230, 180, 60));
            job.append(&s.text[start..end], 0.0, marked);
            pos = end;
        }
        job.append(&s.text[pos..], 0.0, format);
    }
    job
}
