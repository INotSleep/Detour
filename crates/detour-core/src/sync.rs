//! Team operations that go over the network (teams kept in Git).
//!
//! Each takes the store's mutex only between network calls, so other
//! commands keep working while a repository is slow. A per-team lock keeps a
//! background sync and a push after an edit from mirroring one team at once.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};

use crate::store::Store;
use crate::team::git::{GitRemote, Mirror, Remote};
use crate::team::{TeamVault, TeamView, VaultInfo};
use crate::{Error, Result};

static LOCKS: LazyLock<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> = LazyLock::new(Default::default);

fn team_lock(id: &str) -> Arc<tokio::sync::Mutex<()>> {
    LOCKS.lock().unwrap_or_else(|e| e.into_inner()).entry(id.to_string()).or_default().clone()
}

fn locked(store: &Mutex<Store>) -> Result<MutexGuard<'_, Store>> {
    store.lock().map_err(|_| Error::Internal { message: "state lock poisoned".into() })
}

/// Reads a team again. For a Git team: pull, re-read, then push what is pending.
/// Failing to reach the repository is reported in the view, not as an error.
pub async fn sync_team(store: &Mutex<Store>, id: &str, trust_fingerprint: Option<String>) -> Result<TeamView> {
    let job = locked(store)?.git_job(id, trust_fingerprint)?;
    let Some((mirror, remote)) = job else { return locked(store)?.sync_team(id) };
    let lock = team_lock(id);
    let _held = lock.lock().await;

    let pulled = mirror.pull(&remote).await;
    {
        let mut s = locked(store)?;
        if let Ok(p) = &pulled {
            s.pin_known_host(p.new_known_host.clone())?;
        }
        s.sync_team(id)?;
    }
    let outcome = match pulled {
        Ok(_) => mirror.push(&remote).await.map(|p| p.new_known_host),
        Err(e) => Err(e),
    };
    let mut s = locked(store)?;
    let outcome = match outcome {
        Ok(k) => s.pin_known_host(k),
        Err(e) => Err(e),
    };
    s.record_git(id, &outcome);
    s.team_view(id)
}

/// Trusts a repository server's new key in place of the pinned one, then syncs.
pub async fn replace_server_key(store: &Mutex<Store>, id: &str, fingerprint: String) -> Result<TeamView> {
    locked(store)?.forget_git_host_key(id)?;
    sync_team(store, id, Some(fingerprint)).await
}

/// Sends local changes of a Git team (after an edit). Does nothing for a folder team.
pub async fn push_team(store: &Mutex<Store>, id: &str) -> Result<()> {
    let job = locked(store)?.git_job(id, None)?;
    let Some((mirror, remote)) = job else { return Ok(()) };
    let lock = team_lock(id);
    let _held = lock.lock().await;
    let outcome = mirror.push(&remote).await.map(|_| ());
    let mut s = locked(store)?;
    s.record_git(id, &outcome);
    // Whatever was pulled while pushing becomes visible.
    if outcome.is_ok() {
        let _ = s.sync_team(id);
    }
    outcome
}

/// Creates a team in an empty repository.
pub async fn create_git_team(store: &Mutex<Store>, remote: GitRemote, name: &str, my_name: &str, recovery_key: &str, trust_fingerprint: Option<String>) -> Result<TeamView> {
    let (r, (_, staged)) = {
        let s = locked(store)?;
        (s.git_remote(&remote, trust_fingerprint)?, s.staging_dir())
    };
    let (empty, new_known) = Mirror::remote_is_empty(&r).await?;
    locked(store)?.pin_known_host(new_known)?;
    if !empty {
        return Err(Error::FolderNotEmpty);
    }
    let mirror = Mirror::new(&staged);
    locked(store)?.create_team_files(&mirror.files(), name, my_name, recovery_key)?;
    let me = locked(store)?.my_id()?.detour_id;
    let pushed = mirror.push(&r).await;
    if let Err(e) = pushed {
        let _ = std::fs::remove_dir_all(&staged);
        return Err(e);
    }
    joined(store, |s| s.add_git_team(&staged, &me, remote))
}

/// Adds a team just mirrored with its repository, which therefore counts as reached now.
fn joined(store: &Mutex<Store>, add: impl FnOnce(&mut Store) -> Result<TeamView>) -> Result<TeamView> {
    let mut s = locked(store)?;
    let view = add(&mut s)?;
    s.record_git(&view.id, &Ok(()));
    s.team_view(&view.id)
}

/// Clones a repository to see whose team it is. Returns the creator to confirm and a token for `join_git_team`.
pub async fn inspect_git_team(store: &Mutex<Store>, remote: &GitRemote, trust_fingerprint: Option<String>) -> Result<(VaultInfo, String)> {
    let (r, (token, staged)) = {
        let s = locked(store)?;
        (s.git_remote(remote, trust_fingerprint)?, s.staging_dir())
    };
    let mirror = Mirror::new(&staged);
    let pulled = clone(&mirror, &r).await;
    let info = pulled.and_then(|k| {
        locked(store)?.pin_known_host(k)?;
        TeamVault::inspect(mirror.files())
    });
    match info {
        Ok(info) => Ok((info, token)),
        Err(e) => {
            let _ = std::fs::remove_dir_all(&staged);
            Err(e)
        }
    }
}

async fn clone(mirror: &Mirror, r: &Remote) -> Result<Option<crate::model::KnownHost>> {
    let p = mirror.pull(r).await?;
    if !mirror.files().join("vault.json").exists() {
        return Err(Error::NotATeamVault);
    }
    Ok(p.new_known_host)
}

/// Joins the team cloned by `inspect_git_team` once the user confirmed its creator.
pub fn join_git_team(store: &Mutex<Store>, token: &str, root: &str, remote: GitRemote) -> Result<TeamView> {
    let staged = locked(store)?.staged(token)?;
    joined(store, |s| s.add_git_team(&staged, root, remote))
}

/// Uses a recovery key on a team in a repository, then joins it as an owner.
pub async fn recover_git_team(store: &Mutex<Store>, remote: GitRemote, recovery_key: &str, my_name: &str, trust_fingerprint: Option<String>) -> Result<TeamView> {
    let (info, token) = inspect_git_team(store, &remote, trust_fingerprint).await?;
    let (staged, r, me) = {
        let s = locked(store)?;
        (s.staged(&token)?, s.git_remote(&remote, None)?, s.my_id()?.detour_id)
    };
    let mirror = Mirror::new(&staged);
    let recovered = TeamVault::recover(mirror.files(), recovery_key, &me, my_name);
    let pushed = match recovered {
        Ok(()) => mirror.push(&r).await.map(|_| ()),
        Err(e) => Err(e),
    };
    if let Err(e) = pushed {
        let _ = std::fs::remove_dir_all(&staged);
        return Err(e);
    }
    let id = info.id.clone();
    match joined(store, |s| s.add_git_team(&staged, &info.root, remote)) {
        Err(Error::AlreadyMember) => {
            let _ = std::fs::remove_dir_all(&staged);
            sync_team(store, &id, None).await
        }
        other => other,
    }
}
