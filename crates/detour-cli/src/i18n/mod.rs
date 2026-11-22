//! What the command line says, in each language (the system's picks). A language is a
//! file here with a `LANG` of every field; `LANGS` lists them.

use std::sync::OnceLock;

pub struct T {
    pub allow_in_window: &'static str,
    pub master_password: &'static str,
    pub new_master_password: &'static str,
    pub repeat_password: &'static str,
    pub host_password: &'static str,
    pub passphrase: &'static str,
    pub passwords_differ: &'static str,
    pub no_vault: &'static str,
    pub needs_terminal: &'static str,
    pub unlocked: &'static str,
    pub locked: &'static str,
    pub stopped: &'static str,
    pub status_running: &'static str,
    pub status_locked: &'static str,
    pub status_unlocked: &'static str,
    pub status_new: &'static str,
    pub no_hosts: &'static str,
    pub host_added: &'static str,
    pub host_saved: &'static str,
    pub host_removed: &'static str,
    pub unknown_host: &'static str,
    pub which_host: &'static str,
    pub unknown_vault: &'static str,
    pub unknown_key: &'static str,
    pub unknown_snippet: &'static str,
    pub unknown_tunnel: &'static str,
    pub unknown_proxy: &'static str,
    pub unknown_group: &'static str,
    pub group_exists: &'static str,
    pub group_added: &'static str,
    pub group_saved: &'static str,
    pub group_removed: &'static str,
    pub no_group: &'static str,
    pub proxy_added: &'static str,
    pub proxy_removed: &'static str,
    pub proxy_password: &'static str,
    pub proxy_works: fn(u64) -> String,
    pub old_core: &'static str,
    pub unknown_member: &'static str,
    pub new_host_key: &'static str,
    pub changed_host_key: &'static str,
    pub trust_it: &'static str,
    pub replace_it: &'static str,
    pub cancelled: &'static str,
    pub permission_denied: &'static str,
    pub key_saved: &'static str,
    pub key_removed: &'static str,
    pub imported: &'static str,
    pub nothing_to_import: &'static str,
    pub saved_to: &'static str,
    pub file_password: &'static str,
    pub count_hosts: &'static str,
    pub count_keys: &'static str,
    pub count_passwords: &'static str,
    pub count_proxies: &'static str,
    pub count_known_hosts: &'static str,
    pub tunnel_up: &'static str,
    pub tunnel_down: &'static str,
    pub snippet_saved: &'static str,
    pub snippet_removed: &'static str,
    pub synced: &'static str,
    pub sync_off: &'static str,
    pub done: &'static str,
    pub denied: &'static str,
    pub timed_out: &'static str,
    pub core_unreachable: &'static str,
    pub headers_hosts: [&'static str; 4],
    pub headers_keys: [&'static str; 3],
    pub headers_files: [&'static str; 3],
    pub headers_tunnels: [&'static str; 4],
    pub headers_proxies: [&'static str; 4],
    pub headers_groups: [&'static str; 4],
    pub mine_team_only: &'static str,
    pub mine_group_removed: &'static str,
    pub password_word: &'static str,
    pub headers_snippets: [&'static str; 2],
    pub headers_teams: [&'static str; 3],
    pub headers_members: [&'static str; 3],
    pub dir: &'static str,
    pub on: &'static str,
    pub off: &'static str,
    pub you: &'static str,
    pub no_requests: &'static str,
    pub approve_needs_terminal: &'static str,
    pub asks_access: &'static str,
    pub wants_to_run: &'static str,
    pub label_hosts: &'static str,
    pub label_scopes: &'static str,
    pub label_reason: &'static str,
    pub ttl_question: &'static str,
    pub ttl_names: [&'static str; 4],
    pub enter_default: &'static str,
    pub allow_question: &'static str,
    pub allow_command_question: &'static str,
    pub allowed: &'static str,
    pub refused: &'static str,
    pub request_gone: &'static str,
    pub no_agents: &'static str,
    pub headers_agents: [&'static str; 6],
    pub until_revoked: &'static str,
    pub expired: &'static str,
    pub just_now: &'static str,
    pub unknown_agent: &'static str,
    pub which_agent: &'static str,
    pub log_empty: &'static str,
    pub rule_needs_hosts: &'static str,
    pub rule_given_to: &'static str,
    pub rule_token: &'static str,
    pub rule_saved: &'static str,
    pub personal_vault: &'static str,
    pub all_hosts: &'static str,
    pub revoked: &'static str,
    pub unknown_client: &'static str,
    pub headers_mcp: [&'static str; 4],
    pub mcp_added: &'static str,
    pub mcp_outdated: &'static str,
    pub mcp_not_added: &'static str,
    pub mcp_not_installed: &'static str,
    pub mcp_all_added: &'static str,
    pub mcp_no_clients: &'static str,
    pub mcp_added_to: &'static str,
    pub mcp_removed_from: &'static str,
    pub headers_log: [&'static str; 5],
    pub headers_deleted: [&'static str; 4],
    pub colls: [&'static str; 6],
    pub restored_mark: &'static str,
    pub log_team_created: &'static str,
    pub team_renamed: &'static str,
    pub member_added: &'static str,
    pub member_changed: &'static str,
    pub member_removed: &'static str,
    pub this_device: &'static str,
    pub no_deleted: &'static str,
    pub revert_question: &'static str,
    pub nothing_to_revert: &'static str,
    pub no_edit_access: &'static str,
    pub reverted: &'static str,
    pub skipped: &'static str,
    pub undo_with: &'static str,
    pub unknown_entry: &'static str,
    pub which_entry: &'static str,
    pub revert_needs: &'static str,
    pub record_needs_entry: &'static str,
    pub mcp_was_not_in: &'static str,
    pub mcp_restart: &'static str,
    pub https_needs_token: &'static str,
    pub token_needs_https: &'static str,
    pub token_prompt: &'static str,
    pub github_open: &'static str,
    pub github_enter_code: &'static str,
    pub github_signed_in: &'static str,
    pub github_signed_out: &'static str,
    pub github_install: &'static str,
    pub github_no_access: &'static str,
    pub drive_open: &'static str,
    pub drive_paste_code: &'static str,
    pub drive_signed_in: &'static str,
    pub drive_none: &'static str,
    pub drive_expired: &'static str,
    pub drive_unknown: &'static str,
    pub drive_no_folder: &'static str,
    pub recovery_key: &'static str,
    pub recovery_keep: &'static str,
    pub sync_enabled: &'static str,
    pub sync_joined: &'static str,
    pub sync_disabled: &'static str,
    pub code_prompt: &'static str,
    pub device_code: &'static str,
    pub device_code_use: &'static str,
    pub team_created: &'static str,
    pub team_recovery_key: &'static str,
    pub team_recovery_keep: &'static str,
    pub team_creator: &'static str,
    pub team_join_question: &'static str,
    pub team_recovery_prompt: &'static str,
    pub team_joined: &'static str,
    pub team_waiting: &'static str,
    pub teams_elsewhere: &'static str,
    pub stdin_too_big: &'static str,
    pub team_joining: &'static str,
    pub team_cant_join: &'static str,
    pub help_usage: &'static str,
    pub help_commands: &'static str,
    pub help_arguments: &'static str,
    pub help_options: &'static str,
    pub help_help: &'static str,
    pub help_version: &'static str,
    pub arg_unknown_command: &'static str,
    pub arg_unknown_option: &'static str,
    pub arg_missing: &'static str,
    pub arg_bad_value: &'static str,
    pub arg_allowed: &'static str,
    pub arg_conflict: &'static str,
    pub arg_did_you_mean: &'static str,
    pub arg_more: &'static str,
    /// Each command's, option's and argument's text, by key (see `help.rs`).
    pub help: &'static [(&'static str, &'static str)],
    /// The core's errors, by kind.
    pub errors: &'static [(&'static str, &'static str)],
}

mod de;
mod en;
mod es;
mod pt;
mod ru;
mod uk;
mod zh;

/// Each language with the prefixes of its locale.
const LANGS: &[(&str, &T)] = &[("de", &de::LANG), ("es", &es::LANG), ("pt", &pt::LANG), ("ru", &ru::LANG), ("uk", &uk::LANG), ("zh", &zh::LANG), ("en", &en::LANG)];

pub fn t() -> &'static T {
    static LANG: OnceLock<&'static T> = OnceLock::new();
    LANG.get_or_init(|| {
        let env = ["DETOUR_LANG", "LC_ALL", "LC_MESSAGES", "LANG"].iter().find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty() && v != "C" && v != "POSIX"));
        let locale = env.or_else(sys_locale::get_locale).unwrap_or_default().to_lowercase().replace('_', "-");
        LANGS.iter().find(|(code, _)| locale.starts_with(code)).map(|(_, t)| *t).unwrap_or(&en::LANG)
    })
}

/// A text of this language's table, or the English one when it has none.
fn lookup(table: impl Fn(&'static T) -> &'static [(&'static str, &'static str)], key: &str) -> Option<&'static str> {
    let find = |t: &'static T| table(t).iter().find(|(k, _)| *k == key).map(|(_, v)| *v);
    find(t()).or_else(|| find(&en::LANG))
}

/// The help text of a command, option or argument.
pub fn help_text(key: &str) -> Option<&'static str> {
    lookup(|t| t.help, key)
}

/// A core error as a sentence.
pub fn error_text(e: &serde_json::Value) -> String {
    let kind = e["kind"].as_str().unwrap_or("");
    let text = |k: &str| lookup(|t| t.errors, k).unwrap_or("").to_string();
    let field = |f: &str| e[f].as_str().unwrap_or("").to_string();
    match kind {
        "notFound" => format!("{}: {}", text(kind), field("what")),
        "unreachable" => format!("{} {}", text(kind), field("endpoint")),
        "ambiguousHost" => format!("{}: {}", text(kind), e["options"].as_array().into_iter().flatten().filter_map(|o| o.as_str()).collect::<Vec<_>>().join(", ")),
        "driveSignIn" => format!("{} {}", text(kind), field("provider")),
        "authFailed" => t().permission_denied.to_string(),
        "denied" => t().denied.to_string(),
        "timedOut" => t().timed_out.to_string(),
        "disconnected" => t().core_unreachable.to_string(),
        _ => lookup(|t| t.errors, kind).map(str::to_string).unwrap_or_else(|| e["message"].as_str().map(str::to_string).unwrap_or_else(|| if kind.is_empty() { e.to_string() } else { kind.to_string() })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every language has every help text and error the English one has, and no others.
    #[test]
    fn every_language_has_every_text() {
        let keys = |l: &'static [(&'static str, &'static str)]| l.iter().map(|(k, _)| *k).collect::<std::collections::BTreeSet<_>>();
        for (code, t) in LANGS {
            assert_eq!(keys(t.help), keys(en::LANG.help), "help of {code}");
            assert_eq!(keys(t.errors), keys(en::LANG.errors), "errors of {code}");
        }
    }
}
