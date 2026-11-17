//! Vault operations that go over the network: teams and the synced personal vault kept in
//! a Git repository or a cloud drive's folder (`Place`).
//!
//! Each takes the store's mutex only between network calls, so other
//! commands keep working while a repository or drive is slow. A per-team lock keeps a
//! background sync and a push after an edit from mirroring one team at once.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};

use crate::drive::{self, DriveRemote, Storage};
use crate::model::KnownHost;
use crate::personal;
use crate::store::Store;
use crate::team::git::{GitRemote, Mirror, Remote, Synced};
use crate::team::{TeamVault, TeamView, VaultInfo};
use crate::{Error, Result};

/// Where a vault is kept, when not in a folder of this device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Place {
    Git(GitRemote),
    Drive(DriveRemote),
}

impl Place {
    /// The repository or drive folder, as `TeamRef` keeps them.
    pub fn split(p: Option<Place>) -> (Option<GitRemote>, Option<DriveRemote>) {
        match p {
            Some(Place::Git(g)) => (Some(g), None),
            Some(Place::Drive(d)) => (None, Some(d)),
            None => (None, None),
        }
    }

    fn is_drive(&self) -> bool {
        matches!(self, Place::Drive(_))
    }
}

/// A local copy and how to reach what it mirrors.
enum Job {
    Git(Mirror, Remote),
    Drive(drive::Mirror, Arc<dyn Storage>),
}

impl Job {
    fn files(&self) -> PathBuf {
        match self {
            Job::Git(m, _) => m.files(),
            Job::Drive(m, _) => m.files(),
        }
    }

    async fn pull(&self) -> Result<Synced> {
        match self {
            Job::Git(m, r) => m.pull(r).await,
            Job::Drive(m, s) => {
                let (m, s) = (m.clone(), s.clone());
                let d = blocking(move || m.pull(&*s)).await?;
                Ok(Synced { pulled: d.pulled, pushed: d.pushed, new_known_host: None })
            }
        }
    }

    async fn push(&self) -> Result<Synced> {
        match self {
            Job::Git(m, r) => m.push(r).await,
            Job::Drive(m, s) => {
                let (m, s) = (m.clone(), s.clone());
                let d = blocking(move || m.push(&*s)).await?;
                Ok(Synced { pulled: d.pulled, pushed: d.pushed, new_known_host: None })
            }
        }
    }

    /// Nothing there yet? Checked before making a vault there.
    async fn is_empty(&self) -> Result<(bool, Option<KnownHost>)> {
        match self {
            Job::Git(_, r) => Mirror::remote_is_empty(r).await,
            Job::Drive(_, s) => {
                let s = s.clone();
                Ok((blocking(move || drive::Mirror::is_empty(&*s)).await?, None))
            }
        }
    }
}

static LOCKS: LazyLock<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> = LazyLock::new(Default::default);

fn team_lock(id: &str) -> Arc<tokio::sync::Mutex<()>> {
    LOCKS.lock().unwrap_or_else(|e| e.into_inner()).entry(id.to_string()).or_default().clone()
}

/// Renews this device's GitHub token first when it is about to expire.
async fn fresh(store: &Mutex<Store>) {
    let _ = crate::github::renew(store, &crate::github::Endpoints::default()).await;
}

fn locked(store: &Mutex<Store>) -> Result<MutexGuard<'_, Store>> {
    store.lock().map_err(|_| Error::Internal { message: "state lock poisoned".into() })
}

/// Runs blocking work (drive calls; scrypt for pairing codes takes about a second) off the async threads.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(f).await.map_err(|e| Error::Internal { message: e.to_string() })?
}

/// How to reach a place from a local copy in `dir`: a drive with this device's sign-in
/// (renewed first if needed), a repository with this person's keys.
async fn job_at(store: &Mutex<Store>, dir: &Path, place: &Place, trust_fingerprint: Option<String>) -> Result<Job> {
    match place {
        Place::Git(g) => Ok(Job::Git(Mirror::new(dir), locked(store)?.git_remote(g, trust_fingerprint)?)),
        Place::Drive(d) => {
            let token = drive::auth::token(store, d.provider, d.account.as_deref()).await?;
            let d = d.clone();
            let storage = blocking(move || drive::storage(&d, &token)).await?;
            Ok(Job::Drive(drive::Mirror::new(dir), Arc::from(storage)))
        }
    }
}

/// Reads a team again. For a team in Git or a drive: pull, re-read, then push what is pending.
/// Failing to reach the repository or drive is reported in the view, not as an error.
pub async fn sync_team(store: &Mutex<Store>, id: &str, trust_fingerprint: Option<String>) -> Result<TeamView> {
    fresh(store).await;
    let place = locked(store)?.team_place(id)?;
    let Some((dir, place)) = place else { return locked(store)?.sync_team(id) };
    let job = match job_at(store, &dir, &place, trust_fingerprint).await {
        Ok(j) => j,
        // Signed out of the drive, say: the team still shows, with why it can't sync.
        Err(e) if place.is_drive() => {
            let mut s = locked(store)?;
            s.record_git(id, &Err(e));
            s.sync_team(id)?;
            return s.team_view(id);
        }
        Err(e) => return Err(e),
    };
    let lock = team_lock(id);
    let _held = lock.lock().await;

    let pulled = job.pull().await;
    {
        let mut s = locked(store)?;
        if let Ok(p) = &pulled {
            s.pin_known_host(p.new_known_host.clone())?;
        }
        s.sync_team(id)?;
    }
    let outcome = match pulled {
        Ok(_) => job.push().await.map(|p| p.new_known_host),
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

/// Sends local changes of a team in Git or a drive (after an edit). Does nothing for a folder team.
pub async fn push_team(store: &Mutex<Store>, id: &str) -> Result<()> {
    fresh(store).await;
    let place = locked(store)?.team_place(id)?;
    let Some((dir, place)) = place else { return Ok(()) };
    let lock = team_lock(id);
    let _held = lock.lock().await;
    let outcome = match job_at(store, &dir, &place, None).await {
        Ok(job) => job.push().await.map(|_| ()),
        Err(e) => Err(e),
    };
    let mut s = locked(store)?;
    s.record_git(id, &outcome);
    // Whatever was pulled while pushing becomes visible.
    if outcome.is_ok() {
        let _ = s.sync_team(id);
    }
    outcome
}

/// Creates a team in an empty repository or drive folder.
pub async fn create_remote_team(store: &Mutex<Store>, place: Place, name: &str, my_name: &str, recovery_key: &str, trust_fingerprint: Option<String>) -> Result<TeamView> {
    fresh(store).await;
    let (_, staged) = locked(store)?.staging_dir();
    let job = job_at(store, &staged, &place, trust_fingerprint).await?;
    let (empty, new_known) = job.is_empty().await?;
    locked(store)?.pin_known_host(new_known)?;
    if !empty {
        return Err(Error::FolderNotEmpty);
    }
    locked(store)?.create_team_files(&job.files(), name, my_name, recovery_key)?;
    let me = locked(store)?.my_id()?.detour_id;
    let pushed = job.push().await;
    if let Err(e) = pushed {
        let _ = std::fs::remove_dir_all(&staged);
        return Err(e);
    }
    joined(store, |s| s.add_remote_team(&staged, &me, place))
}

/// Creates a team in an empty repository.
pub async fn create_git_team(store: &Mutex<Store>, remote: GitRemote, name: &str, my_name: &str, recovery_key: &str, trust_fingerprint: Option<String>) -> Result<TeamView> {
    create_remote_team(store, Place::Git(remote), name, my_name, recovery_key, trust_fingerprint).await
}

/// Adds a team just mirrored with its repository or drive, which therefore counts as reached now.
fn joined(store: &Mutex<Store>, add: impl FnOnce(&mut Store) -> Result<TeamView>) -> Result<TeamView> {
    let mut s = locked(store)?;
    let view = add(&mut s)?;
    s.record_git(&view.id, &Ok(()));
    s.team_view(&view.id)
}

/// Copies a team from a repository or drive to see whose it is. Returns the creator to
/// confirm and a token for `join_remote_team`.
pub async fn inspect_remote_team(store: &Mutex<Store>, place: &Place, trust_fingerprint: Option<String>) -> Result<(VaultInfo, String)> {
    fresh(store).await;
    let (token, staged) = locked(store)?.staging_dir();
    let info = async {
        let job = job_at(store, &staged, place, trust_fingerprint).await?;
        let k = clone(&job).await?;
        locked(store)?.pin_known_host(k)?;
        TeamVault::inspect(job.files())
    }
    .await;
    match info {
        Ok(info) => Ok((info, token)),
        Err(e) => {
            let _ = std::fs::remove_dir_all(&staged);
            Err(e)
        }
    }
}

/// Clones a repository to see whose team it is.
pub async fn inspect_git_team(store: &Mutex<Store>, remote: &GitRemote, trust_fingerprint: Option<String>) -> Result<(VaultInfo, String)> {
    inspect_remote_team(store, &Place::Git(remote.clone()), trust_fingerprint).await
}

async fn clone(job: &Job) -> Result<Option<KnownHost>> {
    let p = job.pull().await?;
    if !job.files().join("vault.json").exists() {
        return Err(Error::NotATeamVault);
    }
    Ok(p.new_known_host)
}

/// Joins the team copied by `inspect_remote_team` once the user confirmed its creator.
pub fn join_remote_team(store: &Mutex<Store>, token: &str, root: &str, place: Place) -> Result<TeamView> {
    let staged = locked(store)?.staged(token)?;
    joined(store, |s| s.add_remote_team(&staged, root, place))
}

/// Joins the team cloned by `inspect_git_team` once the user confirmed its creator.
pub fn join_git_team(store: &Mutex<Store>, token: &str, root: &str, remote: GitRemote) -> Result<TeamView> {
    join_remote_team(store, token, root, Place::Git(remote))
}

/// Uses a recovery key on a team in a repository or drive, then joins it as an owner.
pub async fn recover_remote_team(store: &Mutex<Store>, place: Place, recovery_key: &str, my_name: &str, trust_fingerprint: Option<String>) -> Result<TeamView> {
    let (info, token) = inspect_remote_team(store, &place, trust_fingerprint).await?;
    let (staged, me) = {
        let s = locked(store)?;
        (s.staged(&token)?, s.my_id()?.detour_id)
    };
    let pushed = async {
        let job = job_at(store, &staged, &place, None).await?;
        TeamVault::recover(job.files(), recovery_key, &me, my_name)?;
        job.push().await.map(|_| ())
    }
    .await;
    if let Err(e) = pushed {
        let _ = std::fs::remove_dir_all(&staged);
        return Err(e);
    }
    let id = info.id.clone();
    match joined(store, |s| s.add_remote_team(&staged, &info.root, place)) {
        Err(Error::AlreadyMember) => {
            let _ = std::fs::remove_dir_all(&staged);
            sync_team(store, &id, None).await
        }
        other => other,
    }
}

/// Uses a recovery key on a team in a repository, then joins it as an owner.
pub async fn recover_git_team(store: &Mutex<Store>, remote: GitRemote, recovery_key: &str, my_name: &str, trust_fingerprint: Option<String>) -> Result<TeamView> {
    recover_remote_team(store, Place::Git(remote), recovery_key, my_name, trust_fingerprint).await
}

/* ---------- the personal vault ---------- */

/// The lock name of the personal vault's sync, apart from any team id.
const PERSONAL: &str = "\0personal";

/// Syncs the personal vault with this person's other devices. For a repository or drive:
/// pull, merge, push. Returns true if records changed here; failing to reach the
/// repository or drive is reported in the status, not as an error.
pub async fn sync_personal(store: &Mutex<Store>, trust_fingerprint: Option<String>) -> Result<bool> {
    fresh(store).await;
    let place = locked(store)?.sync_place()?;
    let Some((dir, place)) = place else { return locked(store)?.sync_personal() };
    let job = match job_at(store, &dir, &place, trust_fingerprint).await {
        Ok(j) => j,
        Err(e) if place.is_drive() => {
            let mut s = locked(store)?;
            s.note_sync(&Err(e));
            return Ok(false);
        }
        Err(e) => return Err(e),
    };
    let lock = team_lock(PERSONAL);
    let _held = lock.lock().await;

    let pulled = job.pull().await;
    let came_in = {
        let mut s = locked(store)?;
        if let Ok(p) = &pulled {
            s.pin_known_host(p.new_known_host.clone())?;
        }
        s.sync_personal()?
    };
    let outcome = match pulled {
        Ok(_) => job.push().await.map(|p| p.new_known_host),
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

/// Sends local changes of the personal vault to its repository or drive (after an edit).
pub async fn push_personal(store: &Mutex<Store>) -> Result<()> {
    fresh(store).await;
    let place = locked(store)?.sync_place()?;
    let Some((dir, place)) = place else { return Ok(()) };
    let lock = team_lock(PERSONAL);
    let _held = lock.lock().await;
    let outcome = match job_at(store, &dir, &place, None).await {
        Ok(job) => job.push().await.map(|_| ()),
        Err(e) => Err(e),
    };
    locked(store)?.note_sync(&outcome);
    outcome
}

/// Starts syncing the personal vault through an empty repository or drive folder.
pub async fn enable_remote_sync(store: &Mutex<Store>, place: Place, trust_fingerprint: Option<String>) -> Result<()> {
    fresh(store).await;
    let (_, staged) = locked(store)?.staging_dir();
    let job = job_at(store, &staged, &place, trust_fingerprint).await?;
    let (empty, new_known) = job.is_empty().await?;
    locked(store)?.pin_known_host(new_known)?;
    if !empty {
        // A synced vault there is this person's from another device, to join instead.
        let synced = clone(&job).await.is_ok() && TeamVault::inspect_personal(job.files()).is_ok();
        let _ = std::fs::remove_dir_all(&staged);
        return Err(if synced { Error::SyncExists } else { Error::FolderNotEmpty });
    }
    let created = locked(store)?.sync_create_files(&job.files());
    let pushed = match created {
        Ok(()) => job.push().await.map(|_| ()),
        Err(e) => Err(e),
    };
    if let Err(e) = pushed {
        let _ = std::fs::remove_dir_all(&staged);
        return Err(e);
    }
    let files = settle(store, &staged)?;
    locked(store)?.sync_open(&files, place)?;
    // This device's records went into the local copy when it was opened.
    let _ = push_personal(store).await;
    Ok(())
}

/// Starts syncing the personal vault through an empty repository.
pub async fn enable_git_sync(store: &Mutex<Store>, remote: GitRemote, trust_fingerprint: Option<String>) -> Result<()> {
    enable_remote_sync(store, Place::Git(remote), trust_fingerprint).await
}

/// Moves a staged copy to where the synced vault's copy lives. Returns its `files`.
fn settle(store: &Mutex<Store>, staged: &Path) -> Result<String> {
    let dir = locked(store)?.sync_dir();
    if dir.exists() {
        std::fs::remove_dir_all(&dir)?;
    }
    std::fs::rename(staged, &dir)?;
    Ok(dir.join("files").display().to_string())
}

/// Joins the personal vault this person syncs through a folder, with a pairing code or recovery key.
pub async fn join_folder_sync(store: &Mutex<Store>, path: String, code: String) -> Result<()> {
    let meta = TeamVault::inspect_personal(&path)?;
    let p = path.clone();
    let identity = blocking(move || personal::redeem(Path::new(&p), &meta, &code)).await?;
    locked(store)?.sync_join(&path, &identity, None)
}

/// Joins the personal vault this person syncs through a repository or drive, with a pairing code or recovery key.
pub async fn join_remote_sync(store: &Mutex<Store>, place: Place, code: String, trust_fingerprint: Option<String>) -> Result<()> {
    fresh(store).await;
    let (_, staged) = locked(store)?.staging_dir();
    let opened = async {
        let job = job_at(store, &staged, &place, trust_fingerprint).await?;
        let k = clone(&job).await.map_err(|e| if matches!(e, Error::NotATeamVault) { Error::NotASyncVault } else { e })?;
        locked(store)?.pin_known_host(k)?;
        let meta = TeamVault::inspect_personal(job.files())?;
        let files = job.files();
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
    let joined = locked(store)?.sync_join(&files, &identity, Some(place));
    if let Err(e) = joined {
        let _ = std::fs::remove_dir_all(locked(store)?.sync_dir());
        return Err(e);
    }
    let _ = push_personal(store).await;
    Ok(())
}

/// Joins the personal vault this person syncs through a repository.
pub async fn join_git_sync(store: &Mutex<Store>, remote: GitRemote, code: String, trust_fingerprint: Option<String>) -> Result<()> {
    join_remote_sync(store, Place::Git(remote), code, trust_fingerprint).await
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

/// Joins here the teams in Git or a drive this person joined on another device: where they
/// are and their creator come with the link, so nothing needs asking (a team in a drive
/// waits until this device is signed in to that drive). Returns true if any was joined.
pub async fn join_linked_teams(store: &Mutex<Store>) -> bool {
    let Ok((git, drives)) = locked(store).and_then(|s| Ok((s.pending_git_teams()?, s.pending_drive_teams()?))) else { return false };
    let pending = git.into_iter().map(|(id, root, r)| (id, root, Place::Git(r))).chain(drives.into_iter().map(|(id, root, d)| (id, root, Place::Drive(d))));
    let mut joined = false;
    for (id, root, place) in pending {
        let result = join_linked(store, &id, &root, place).await;
        if let Ok(mut s) = locked(store) {
            s.note_link_error(&id, result.as_ref().err());
        }
        joined |= result.is_ok();
    }
    joined
}

async fn join_linked(store: &Mutex<Store>, id: &str, root: &str, place: Place) -> Result<TeamView> {
    let (info, token) = inspect_remote_team(store, &place, None).await?;
    if info.id != id || info.root != root {
        if let Ok(dir) = locked(store)?.staged(&token) {
            let _ = std::fs::remove_dir_all(dir);
        }
        return Err(Error::NotATeamVault);
    }
    join_remote_team(store, &token, root, place)
}

/// Joins here, through a drive folder this person picked, a team joined on another device
/// (where it may be a folder synced by the drive's own app).
pub async fn join_linked_drive_team(store: &Mutex<Store>, team_id: &str, folder: DriveRemote) -> Result<TeamView> {
    let link = locked(store)?.team_link(team_id)?;
    let joined = join_linked(store, team_id, &link.root, Place::Drive(folder)).await;
    if let Ok(mut s) = locked(store) {
        s.note_link_error(team_id, joined.as_ref().err());
    }
    joined
}
