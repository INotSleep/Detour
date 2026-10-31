//! What the core does by itself, with or without a window: syncing the personal
//! vault and the teams, telling clients how tunnels are doing, locking after
//! inactivity, and ending once it is locked and nobody needs it.

use std::sync::Arc;
use std::time::Duration;

use detour_core::sync;
use serde_json::{Value, json};

use crate::Core;

const SYNC_EVERY: Duration = Duration::from_secs(20);
/// A locked core with no sessions, tunnels or clients ends after this long.
const IDLE_EXIT: Duration = Duration::from_secs(300);

pub(crate) fn spawn(core: Arc<Core>) {
    tokio::spawn(personal_sync(core.clone()));
    tokio::spawn(team_sync(core.clone()));
    tokio::spawn(tunnel_status(core.clone()));
    tokio::spawn(lock_and_exit(core));
}

fn unlocked(core: &Core) -> bool {
    core.store.lock().is_ok_and(|s| s.is_unlocked())
}

async fn personal_sync(core: Arc<Core>) {
    let mut tick = tokio::time::interval(SYNC_EVERY);
    loop {
        tick.tick().await;
        let enabled = core.store.lock().ok().and_then(|s| s.sync_status().ok()).is_some_and(|s| s.enabled);
        if !unlocked(&core) || !enabled {
            continue;
        }
        let changed = sync::sync_personal(&core.store, None).await.unwrap_or(false);
        // Teams in Git joined on another device get joined here too.
        let joined = sync::join_linked_teams(&core.store).await;
        if let Ok(status) = core.store.lock().map(|s| s.sync_status()) {
            core.emit("sync.status", json!(status.ok()));
        }
        if changed || joined {
            core.emit("data.changed", json!({ "by": 0, "method": "sync_now" }));
        }
    }
}

async fn team_sync(core: Arc<Core>) {
    let mut tick = tokio::time::interval(SYNC_EVERY);
    loop {
        tick.tick().await;
        if !unlocked(&core) {
            continue;
        }
        let ids: Vec<String> = core.store.lock().map(|s| s.teams().into_iter().map(|t| t.id).collect()).unwrap_or_default();
        for id in ids {
            if let Ok(view) = sync::sync_team(&core.store, &id, None).await {
                core.emit("team.synced", json!(view));
            }
        }
    }
}

async fn tunnel_status(core: Arc<Core>) {
    let mut last = Value::Null;
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    loop {
        tick.tick().await;
        let now = json!(core.tunnels.statuses());
        if now != last {
            core.emit("tunnels.status", now.clone());
            last = now;
        }
    }
}

async fn lock_and_exit(core: Arc<Core>) {
    let mut tick = tokio::time::interval(Duration::from_secs(15));
    loop {
        tick.tick().await;
        let minutes = core.settings.lock().unwrap().auto_lock_minutes;
        if minutes > 0 && unlocked(&core) && core.idle_for() > Duration::from_secs(minutes as u64 * 60) {
            core.lock();
        }
        let busy = unlocked(&core) || !core.sessions.is_empty() || !core.tunnels.statuses().is_empty() || core.has_clients();
        if !busy && core.idle_for() > IDLE_EXIT {
            core.quit.notify_one();
            return;
        }
    }
}
