//! Read state shared with WeeChat. What is read here is read there: opening a
//! buffer clears its entry in WeeChat's hotlist, leaving it moves WeeChat's read
//! marker to its end, and a buffer read in WeeChat is read here. WeeChat can also
//! follow along, switching to the buffer that is opened here.
//!
//! The relay says when a buffer enters the hotlist but not when it leaves, so the
//! hotlist is asked for every few seconds and a buffer that was in it and no longer
//! is was read in WeeChat. A buffer that never entered it (the one WeeChat itself
//! has open) is left alone: its lines are counted here as they arrive.

use std::time::{Duration, Instant};

use crate::ui::app::WeeChatApp;

/// The open buffer is marked read in WeeChat at most this often while lines arrive.
const EVERY: Duration = Duration::from_secs(5);
/// And never sooner than this after a line, so a burst is one request.
const SETTLE: Duration = Duration::from_millis(1500);
/// How often WeeChat's hotlist is asked for.
const POLL: Duration = Duration::from_secs(10);

impl WeeChatApp {
    fn on_weechat(&self, buffer_id: &str) -> bool {
        self.saved_state_key(buffer_id).is_some_and(|(_, weechat)| weechat)
    }

    /// Whether reading `buffer_id` is told to its server: always, except on a
    /// WeeChat relay with the setting off.
    pub(crate) fn syncs_read(&self, buffer_id: &str) -> bool {
        self.sync_read_weechat || !self.on_weechat(buffer_id)
    }

    /// A buffer on WeeChat was left: its read marker goes after its last line.
    pub(crate) fn read_sync_left(&self, buffer_id: &str) {
        if !self.sync_read_weechat || !self.on_weechat(buffer_id) {
            return;
        }
        if let Some((client, raw_id)) = self.client_for_buffer(buffer_id) {
            client.send_message(&raw_id, "/buffer set unread");
        }
    }

    /// A buffer on WeeChat was opened: WeeChat switches to it too, when asked to.
    pub(crate) fn follow_in_weechat(&self, buffer_id: &str) {
        if !self.follow_weechat || !self.on_weechat(buffer_id) {
            return;
        }
        let Some(buffer) = self.buffer_by_id(buffer_id) else { return };
        // The name as WeeChat knows it, without the connection before it.
        let name = buffer.full_name.split_once('/').map_or(buffer.full_name.as_str(), |(_, name)| name);
        if name.is_empty() {
            return;
        }
        if let Some((client, raw_id)) = self.client_for_buffer(buffer_id) {
            client.send_message(&raw_id, &format!("/buffer {name}"));
        }
    }

    /// A line arrived in the open buffer: WeeChat is told it was read, soon.
    pub(crate) fn read_sync_soon(&mut self) {
        if !self.sync_read_weechat || self.read_sync_due.is_some() {
            return;
        }
        let since = self.read_synced_at.map_or(EVERY, |at| at.elapsed());
        self.read_sync_due = Some(Instant::now() + EVERY.saturating_sub(since).max(SETTLE));
    }

    /// Called every frame. `visible` is whether the user can see the chat: lines
    /// that arrive behind another window haven't been read.
    pub(crate) fn read_sync_tick(&mut self, ctx: &egui::Context, visible: bool) {
        if self.sync_read_weechat && self.hotlist_polled_at.is_none_or(|at| at.elapsed() >= POLL) {
            let mut asked = false;
            for conn in self.connections.iter().filter(|c| c.backend_type == crate::ui::app::BackendType::WeeChat && c.client.is_connected()) {
                conn.client.poll_hotlist();
                asked = true;
            }
            self.hotlist_polled_at = Some(Instant::now());
            if asked {
                ctx.request_repaint_after(POLL);
            }
        }
        let Some(due) = self.read_sync_due else { return };
        let now = Instant::now();
        if now < due {
            ctx.request_repaint_after(due - now);
            return;
        }
        self.read_sync_due = None;
        if !visible || !self.sync_read_weechat {
            return;
        }
        let Some(id) = self.selected_buffer_id.clone().filter(|id| self.on_weechat(id)) else { return };
        if let Some((client, raw_id)) = self.client_for_buffer(&id) {
            if client.is_connected() {
                client.mark_read(&raw_id);
                self.read_synced_at = Some(now);
            }
        }
    }

    /// WeeChat's whole hotlist for a connection came in: the buffers that were in the
    /// last one and aren't in this one were read in WeeChat.
    pub(crate) fn hotlist_listed(&mut self, conn_prefix: &str, listed: std::collections::HashSet<String>) {
        let ours = format!("{conn_prefix}/");
        let gone: Vec<String> = self.weechat_hotlist.iter()
            .filter(|id| id.starts_with(&ours) && !listed.contains(*id))
            .cloned()
            .collect();
        self.weechat_hotlist.retain(|id| !id.starts_with(&ours));
        self.weechat_hotlist.extend(listed);
        for id in gone {
            self.read_in_weechat(&id);
        }
    }

    /// WeeChat dropped a buffer from its hotlist: it was read there, so it is read here.
    pub(crate) fn read_in_weechat(&mut self, buffer_id: &str) {
        if !self.sync_read_weechat || self.selected_buffer_id.as_deref() == Some(buffer_id) {
            return;
        }
        if let Some(buffer) = self.buffer_by_id_mut(buffer_id) {
            buffer.activity = crate::relay::models::BufferActivity::None;
            buffer.unread_count = 0;
            if let Some(last) = buffer.messages.back() {
                buffer.last_read_id = Some(last.id.clone());
            }
        }
        self.mentions_seen_in(buffer_id);
    }
}
