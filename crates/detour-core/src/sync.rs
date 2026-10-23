//! Team operations that go over the network (teams kept in Git).
//!
//! Each takes the store's mutex only between network calls, so other
//! commands keep working while a repository is slow. A per-team lock keeps a
//! background sync and a push after an edit from mirroring one team at once.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};

use crate::personal;
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

/* ---------- the personal vault ---------- */

/// The lock name of the personal vault's sync, apart from any team id.
const PERSONAL: &str = "\0personal";

/// Runs blocking work (scrypt for pairing codes takes about a second) off the async threads.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(f).await.map_err(|e| Error::Internal { message: e.to_string() })?
}

/// Syncs the personal vault with this person's other devices. For a repository:
/// pull, merge, push. Returns true if records changed here; failing to reach
/// the repository is reported in the status, not as an error.
pub async fn sync_personal(store: &Mutex<Store>, trust_fingerprint: Option<String>) -> Result<bool> {
    let job = locked(store)?.sync_git_job(trust_fingerprint)?;
    let Some((mirror, remote)) = job else { return locked(store)?.sync_personal() };
    let lock = team_lock(PERSONAL);
    let _held = lock.lock().await;

    let pulled = mirror.pull(&remote).await;
    let came_in = {
        let mut s = locked(store)?;
        if let Ok(p) = &pulled {
            s.pin_known_host(p.new_known_host.clone())?;
        }
        s.sync_personal()?
    };
    let outcome = match pulled {
        Ok(_) => mirror.push(&remote).await.map(|p| p.new_known_host),
        Err(e) => Err(e),
    };
    let mut s = locked(store)?;
    let outcome = match outcome {
        Ok(k) => s.pin_known_host(k),
        Err(e) => Err(e),
    };
    s.note_sync(&outcome);
    Ok(came_in)
}

/// Sends local changes of the personal vault to its repository (after an edit).
pub async fn push_personal(store: &Mutex<Store>) -> Result<()> {
    let job = locked(store)?.sync_git_job(None)?;
    let Some((mirror, remote)) = job else { return Ok(()) };
    let lock = team_lock(PERSONAL);
    let _held = lock.lock().await;
    let outcome = mirror.push(&remote).await.map(|_| ());
    locked(store)?.note_sync(&outcome);
    outcome
}

/// Starts syncing the personal vault through an empty repository.
pub async fn enable_git_sync(store: &Mutex<Store>, remote: GitRemote, trust_fingerprint: Option<String>) -> Result<()> {
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
    let created = locked(store)?.sync_create_files(&mirror.files());
    let pushed = match created {
        Ok(()) => mirror.push(&r).await.map(|_| ()),
        Err(e) => Err(e),
    };
    if let Err(e) = pushed {
        let _ = std::fs::remove_dir_all(&staged);
        return Err(e);
    }
    let files = settle(store, &staged)?;
    locked(store)?.sync_open(&files, remote)?;
    // This device's records went into the local copy when it was opened.
    let _ = push_personal(store).await;
    Ok(())
}

/// Moves a staged repository copy to where the synced vault's copy lives. Returns its `files`.
fn settle(store: &Mutex<Store>, staged: &std::path::Path) -> Result<String> {
    let dir = locked(store)?.sync_dir();
    if dir.exists() {
        std::fs::remove_dir_all(&dir)?;
    }
    std::fs::rename(staged, &dir)?;
    Ok(Mirror::new(&dir).files().display().to_string())
}

/// Joins the personal vault this person syncs through a folder, with a pairing code or recovery key.
pub async fn join_folder_sync(store: &Mutex<Store>, path: String, code: String) -> Result<()> {
    let meta = TeamVault::inspect_personal(&path)?;
    let p = path.clone();
    let identity = blocking(move || personal::redeem(std::path::Path::new(&p), &meta, &code)).await?;
    locked(store)?.sync_join(&path, &identity, None)
}

/// Joins the personal vault this person syncs through a repository, with a pairing code or recovery key.
pub async fn join_git_sync(store: &Mutex<Store>, remote: GitRemote, code: String, trust_fingerprint: Option<String>) -> Result<()> {
    let (r, (_, staged)) = {
        let s = locked(store)?;
        (s.git_remote(&remote, trust_fingerprint)?, s.staging_dir())
    };
    let mirror = Mirror::new(&staged);
    let opened = async {
        let k = clone(&mirror, &r).await.map_err(|e| if matches!(e, Error::NotATeamVault) { Error::NotASyncVault } else { e })?;
        locked(store)?.pin_known_host(k)?;
        let meta = TeamVault::inspect_personal(mirror.files())?;
        let files = mirror.files();
        blocking(move || personal::redeem(&files, &meta, &code)).await
    }
    .await;
    let identity = match opened {
        Ok(i) => i,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&staged);
            return Err(e);
        }
    };
    let files = settle(store, &staged)?;
    let joined = locked(store)?.sync_join(&files, &identity, Some(remote));
    if let Err(e) = joined {
        let _ = std::fs::remove_dir_all(locked(store)?.sync_dir());
        return Err(e);
    }
    let _ = push_personal(store).await;
    Ok(())
}

/// Makes a pairing code for another device of this person, and sends it where that device will look.
pub async fn add_device(store: &Mutex<Store>) -> Result<personal::Pairing> {
    let (path, id, me) = locked(store)?.sync_offer_parts()?;
    let pairing = blocking(move || personal::offer(&path, &id, &me)).await?;
    let _ = push_personal(store).await;
    Ok(pairing)
}

/// Trusts the sync repository server's new key in place of the pinned one, then syncs.
pub async fn replace_sync_server_key(store: &Mutex<Store>, fingerprint: String) -> Result<bool> {
    locked(store)?.sync_forget_host_key()?;
    sync_personal(store, Some(fingerprint)).await
}
