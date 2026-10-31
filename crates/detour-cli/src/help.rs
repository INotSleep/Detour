//! The command line's help in the person's language: clap's own words and each
//! command's and option's text come from i18n, not from the code.

use clap::builder::styling::Style;
use clap::error::{ContextKind, ErrorKind};
use clap::{Arg, ArgAction, Command, CommandFactory};

use crate::i18n::t;

pub fn command() -> Command {
    let t = t();
    let root = crate::Cli::command()
        .bin_name("detour")
        .disable_version_flag(true)
        .arg(Arg::new("version").short('V').long("version").action(ArgAction::Version).help(t.help_version))
        .arg(Arg::new("help").short('h').long("help").action(ArgAction::Help).global(true).help(t.help_help));
    localize(root, "")
}

/// A text by its key: "hosts add" for a command, "hosts add --address" or
/// "hosts add <name>" for its options and arguments.
fn text(key: &str) -> Option<&'static str> {
    t().help.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
}

fn localize(mut cmd: Command, path: &str) -> Command {
    let t = t();
    if let Some(about) = text(path) {
        cmd = cmd.about(about);
    }
    let heading = Style::new().bold().underline();
    cmd = cmd
        .disable_help_flag(true)
        .subcommand_help_heading(t.help_commands)
        .help_template(format!("{{about-with-newline}}\n{heading}{}{heading:#} {{usage}}\n\n{{all-args}}", t.help_usage));

    let args: Vec<(String, bool, Option<String>)> = cmd.get_arguments().map(|a| (a.get_id().to_string(), a.is_positional(), a.get_long().map(str::to_owned))).collect();
    // In the order they are declared; the shared options (the root's) after a command's own.
    let first = if path.is_empty() { 1000 } else { 0 };
    for (i, (id, positional, long)) in args.into_iter().enumerate() {
        let part = match long {
            Some(l) if !positional => format!("--{l}"),
            _ => format!("<{id}>"),
        };
        // The text for this command, or the one its options and arguments share elsewhere.
        let help = text(&format!("{path} {part}").trim_start().to_owned()).or_else(|| text(&part));
        cmd = cmd.mut_arg(id, |a| {
            // Defaults and possible values, where they matter, are in the text itself.
            let a = a.help_heading(if positional { t.help_arguments } else { t.help_options }).display_order(first + i);
            let a = if a.get_action().takes_values() { a.hide_default_value(true).hide_possible_values(true) } else { a };
            match help {
                Some(h) => a.help(h),
                None => a,
            }
        });
    }

    let subs: Vec<String> = cmd.get_subcommands().map(|s| s.get_name().to_owned()).collect();
    for name in subs {
        let sub = format!("{path} {name}").trim_start().to_owned();
        cmd = cmd.mut_subcommand(name, |s| localize(s, &sub));
    }
    cmd
}

/// The command line, parsed; a mistake in it is told in the person's language.
pub fn parse() -> crate::Cli {
    use clap::FromArgMatches;
    command().try_get_matches().and_then(|m| crate::Cli::from_arg_matches(&m)).unwrap_or_else(|e| {
        let Some(msg) = mistake(&e) else { e.exit() };
        let t = t();
        eprintln!("detour: {msg}");
        if let Some(usage) = e.get(ContextKind::Usage) {
            eprintln!("\n{}", usage.to_string().replacen("Usage:", t.help_usage, 1));
        }
        eprintln!("\n{}", t.arg_more);
        std::process::exit(2)
    })
}

/// What is wrong, for the mistakes people make; clap's own words for the rest.
fn mistake(e: &clap::Error) -> Option<String> {
    let t = t();
    let get = |k| e.get(k).map(|v| v.to_string()).unwrap_or_default();
    let hint = |k| e.get(k).map(|v| format!(" ({} {v})", t.arg_did_you_mean)).unwrap_or_default();
    let (arg, value) = (get(ContextKind::InvalidArg), get(ContextKind::InvalidValue));
    Some(match e.kind() {
        ErrorKind::InvalidSubcommand => format!("{}: {}{}", t.arg_unknown_command, get(ContextKind::InvalidSubcommand), hint(ContextKind::SuggestedSubcommand)),
        ErrorKind::UnknownArgument => format!("{}: {arg}{}", t.arg_unknown_option, hint(ContextKind::SuggestedArg)),
        ErrorKind::MissingRequiredArgument => format!("{}: {arg}", t.arg_missing),
        ErrorKind::InvalidValue if value.is_empty() => format!("{}: {arg}", t.arg_missing),
        ErrorKind::InvalidValue | ErrorKind::ValueValidation => match e.get(ContextKind::ValidValue) {
            Some(valid) => format!("{} {value} ({arg}); {}: {valid}", t.arg_bad_value, t.arg_allowed),
            None => format!("{} {value} ({arg})", t.arg_bad_value),
        },
        ErrorKind::ArgumentConflict => format!("{arg}, {}: {}", get(ContextKind::PriorArg), t.arg_conflict),
        _ => return None,
    })
}
