use anyhow::{Context, Result};
use chrono::{DateTime, FixedOffset};
use clap::{Args, Parser, Subcommand};
use directories::BaseDirs;
use indicatif::{ProgressBar, ProgressStyle};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::Duration,
};

/// Himalaya cache CLI.
#[derive(Parser)]
#[command(name = "himalaya-cache")]
#[command(about = "Cache data from the himalaya CLI", version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Sync accounts, folders, and messages from himalaya.
    Sync(SyncArgs),
    /// Read cached folder data.
    Folder(FolderArgs),
    /// Read cached message data.
    Message(MessageArgs),
    /// Read cached envelope data.
    Envelope(EnvelopeArgs),
}

#[derive(Args)]
struct SyncArgs {
    /// Sync a single account by name.
    #[arg(long)]
    account: Option<String>,
    /// Sync a single folder by name (requires --account).
    #[arg(long)]
    folder: Option<String>,
}

#[derive(Subcommand)]
enum FolderCommand {
    /// List cached folders for an account.
    List(FolderListArgs),
}

#[derive(Args)]
struct FolderArgs {
    #[command(subcommand)]
    command: FolderCommand,
}

#[derive(Args)]
struct FolderListArgs {
    /// Account name to read cached folders for.
    #[arg(long)]
    account: String,
}

#[derive(Subcommand)]
enum MessageCommand {
    /// Read a cached message by id.
    Read(MessageReadArgs),
}

#[derive(Args)]
struct MessageArgs {
    #[command(subcommand)]
    command: MessageCommand,
}

#[derive(Args)]
struct MessageReadArgs {
    /// Account name to read cached message for.
    #[arg(long)]
    account: String,
    /// Folder name to read cached message for.
    #[arg(long)]
    folder: String,
    /// Message id to read.
    id: String,
}

#[derive(Subcommand)]
enum EnvelopeCommand {
    /// List cached envelopes for an account and folder.
    List(EnvelopeListArgs),
}

#[derive(Args)]
struct EnvelopeArgs {
    #[command(subcommand)]
    command: EnvelopeCommand,
}

#[derive(Args)]
struct EnvelopeListArgs {
    /// Account name to read cached envelopes for.
    #[arg(long)]
    account: String,
    /// Folder name to read cached envelopes for.
    #[arg(long)]
    folder: String,
}

/// Account entry from `himalaya account list -o json`.
#[derive(Debug, Deserialize, Serialize)]
struct Account {
    name: String,
    backend: Option<String>,
    default: Option<bool>,
}

/// Folder entry from `himalaya folder list -o json`.
#[derive(Debug, Deserialize, Serialize)]
struct Folder {
    name: String,
    desc: Option<String>,
}

/// Envelope entry from `himalaya envelope list -o json`.
#[derive(Debug, Deserialize, Serialize)]
struct Envelope {
    id: String,
    flags: Option<Vec<String>>,
    subject: Option<String>,
    from: Option<Contact>,
    to: Option<Contact>,
    date: Option<String>,
    has_attachment: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize)]
struct Contact {
    name: Option<String>,
    addr: Option<String>,
}

fn main() -> Result<()> {
    let raw_args: Vec<String> = std::env::args().collect();
    if raw_args.len() <= 1 {
        let cli = Cli::parse();
        return match cli.command {
            Commands::Sync(args) => run_sync(args),
            Commands::Folder(args) => run_folder(args),
            Commands::Message(args) => run_message(args),
            Commands::Envelope(args) => run_envelope(args),
        };
    }

    if let Some(result) = try_run_internal(&raw_args[1..]) {
        return result;
    }

    run_himalaya_passthrough(&raw_args[1..])
}

fn try_run_internal(args: &[String]) -> Option<Result<()>> {
    let command = args.first()?.as_str();
    match command {
        "sync" => Some(parse_and_run_sync(&args[1..])),
        "folder" => match args.get(1).map(String::as_str) {
            Some("list") => Some(parse_and_run_folder_list(&args[2..])),
            _ => None,
        },
        "message" => match args.get(1).map(String::as_str) {
            Some("read") => Some(parse_and_run_message_read(&args[2..])),
            _ => None,
        },
        "envelope" => match args.get(1).map(String::as_str) {
            Some("list") => Some(parse_and_run_envelope_list(&args[2..])),
            _ => None,
        },
        _ => None,
    }
}

fn run_himalaya_passthrough(args: &[String]) -> Result<()> {
    let status = Command::new(himalaya_path()?)
        .args(args)
        .status()
        .with_context(|| "run himalaya")?;
    if status.success() {
        Ok(())
    } else {
        Err(anyhow::anyhow!("himalaya exited with status {}", status))
    }
}

fn parse_and_run_sync(args: &[String]) -> Result<()> {
    let (flags, _) = parse_args(args, &["--account", "--folder"], 0);
    let sync_args = SyncArgs {
        account: flags.get("--account").cloned(),
        folder: flags.get("--folder").cloned(),
    };
    run_sync(sync_args)
}

fn parse_and_run_folder_list(args: &[String]) -> Result<()> {
    let (flags, _) = parse_args(args, &["--account"], 0);
    let account = flags
        .get("--account")
        .cloned()
        .context("--account is required")?;
    list_cached_folders(FolderListArgs { account })
}

fn parse_and_run_message_read(args: &[String]) -> Result<()> {
    let (flags, positionals) = parse_args(args, &["--account", "--folder"], 1);
    let account = flags
        .get("--account")
        .cloned()
        .context("--account is required")?;
    let folder = flags
        .get("--folder")
        .cloned()
        .context("--folder is required")?;
    let id = positionals
        .first()
        .cloned()
        .context("message id is required")?;
    read_cached_message(MessageReadArgs {
        account,
        folder,
        id,
    })
}

fn parse_and_run_envelope_list(args: &[String]) -> Result<()> {
    let (flags, _) = parse_args(args, &["--account", "--folder"], 0);
    let account = flags
        .get("--account")
        .cloned()
        .context("--account is required")?;
    let folder = flags
        .get("--folder")
        .cloned()
        .context("--folder is required")?;
    list_cached_envelopes(EnvelopeListArgs { account, folder })
}

fn parse_args(
    args: &[String],
    known_flags: &[&str],
    required_positionals: usize,
) -> (HashMap<String, String>, Vec<String>) {
    let mut flags = HashMap::new();
    let mut positionals = Vec::new();
    let mut index = 0;
    while index < args.len() {
        let token = &args[index];
        if token.starts_with('-') {
            if known_flags.contains(&token.as_str()) {
                if let Some(value) = args.get(index + 1) {
                    flags.insert(token.clone(), value.clone());
                    index += 2;
                } else {
                    index += 1;
                }
            } else {
                let remaining_non_flags = count_remaining_non_flags(args, index + 1);
                if remaining_non_flags > required_positionals {
                    if let Some(value) = args.get(index + 1) {
                        if !value.starts_with('-') {
                            index += 2;
                            continue;
                        }
                    }
                }
                index += 1;
            }
        } else {
            positionals.push(token.clone());
            index += 1;
        }
    }
    (flags, positionals)
}

fn count_remaining_non_flags(args: &[String], start: usize) -> usize {
    args.iter()
        .skip(start)
        .filter(|value| !value.starts_with('-'))
        .count()
}

/// Handle cached folder subcommands.
fn run_folder(args: FolderArgs) -> Result<()> {
    match args.command {
        FolderCommand::List(args) => list_cached_folders(args),
    }
}

/// Handle cached message subcommands.
fn run_message(args: MessageArgs) -> Result<()> {
    match args.command {
        MessageCommand::Read(args) => read_cached_message(args),
    }
}

/// Handle cached envelope subcommands.
fn run_envelope(args: EnvelopeArgs) -> Result<()> {
    match args.command {
        EnvelopeCommand::List(args) => list_cached_envelopes(args),
    }
}

/// Print cached folders for the given account.
fn list_cached_folders(args: FolderListArgs) -> Result<()> {
    let cache_dir = cache_dir()?;
    let folders_path = cache_dir
        .join("folders")
        .join(format!("{}.json", args.account));
    let contents = fs::read_to_string(&folders_path)
        .with_context(|| format!("read {}", folders_path.display()))?;
    println!("{contents}");
    Ok(())
}

/// Print a cached message content for the given account, folder, and id.
fn read_cached_message(args: MessageReadArgs) -> Result<()> {
    let cache_dir = cache_dir()?;
    let message_path = cache_dir
        .join("messages")
        .join(&args.account)
        .join(&args.folder)
        .join(format!("{}.eml", args.id));
    let contents =
        fs::read(&message_path).with_context(|| format!("read {}", message_path.display()))?;
    let normalized = String::from_utf8_lossy(&contents).replace("\r\n", "\n");
    let wrapped = serde_json::to_string(&normalized).context("serialize message")?;
    let mut stdout = io::stdout();
    stdout
        .write_all(wrapped.as_bytes())
        .with_context(|| "write message to stdout")?;
    Ok(())
}

/// Print cached envelopes sorted by date (ascending).
fn list_cached_envelopes(args: EnvelopeListArgs) -> Result<()> {
    let cache_dir = cache_dir()?;
    let meta_dir = cache_dir
        .join("meta")
        .join(&args.account)
        .join(&args.folder);

    let mut envelopes = Vec::new();
    for entry in fs::read_dir(&meta_dir).with_context(|| format!("read {}", meta_dir.display()))? {
        let entry = entry.with_context(|| format!("read entry in {}", meta_dir.display()))?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let data = fs::read(&path).with_context(|| format!("read {}", path.display()))?;
        let envelope: Envelope =
            serde_json::from_slice(&data).with_context(|| format!("parse {}", path.display()))?;
        envelopes.push(envelope);
    }

    envelopes.sort_by(|left, right| {
        let left_date = parse_envelope_date(left);
        let right_date = parse_envelope_date(right);
        right_date.cmp(&left_date)
    });

    let output = serde_json::to_string_pretty(&envelopes).context("serialize envelopes")?;
    println!("{output}");
    Ok(())
}

fn parse_envelope_date(envelope: &Envelope) -> Option<DateTime<FixedOffset>> {
    envelope
        .date
        .as_deref()
        .and_then(|value| DateTime::parse_from_str(value, "%Y-%m-%d %H:%M%:z").ok())
}

/// Perform a cache sync, optionally scoped to account and folder.
fn run_sync(args: SyncArgs) -> Result<()> {
    if args.folder.is_some() && args.account.is_none() {
        anyhow::bail!("--folder requires --account");
    }

    let cache_dir = cache_dir()?;
    fs::create_dir_all(&cache_dir)
        .with_context(|| format!("create cache dir {}", cache_dir.display()))?;

    let account_names = match args.account.as_deref() {
        Some(account_name) => vec![account_name.to_string()],
        None => {
            let accounts: Vec<Account> = match run_himalaya_json(&["account", "list", "-o", "json"])
            {
                Ok(accounts) => accounts,
                Err(err) => {
                    return Err(err).context("fetch account list");
                }
            };
            let accounts_path = cache_dir.join("accounts.json");
            write_json(&accounts_path, &accounts)
                .with_context(|| format!("write {}", accounts_path.display()))?;
            accounts.into_iter().map(|account| account.name).collect()
        }
    };

    for account_name in account_names {
        let folder_names = match args.folder.as_deref() {
            Some(folder_name) => vec![folder_name.to_string()],
            None => {
                let folders: Vec<Folder> = match run_himalaya_json(&[
                    "folder",
                    "list",
                    "--account",
                    &account_name,
                    "-o",
                    "json",
                ]) {
                    Ok(folders) => folders,
                    Err(err) => {
                        eprintln!(
                            "warning: failed to fetch folders for account {}: {:#}",
                            account_name, err
                        );
                        continue;
                    }
                };

                let folders_path = cache_dir
                    .join("folders")
                    .join(format!("{}.json", &account_name));
                write_json(&folders_path, &folders)
                    .with_context(|| format!("write {}", folders_path.display()))?;
                folders.into_iter().map(|folder| folder.name).collect()
            }
        };

        for folder_name in folder_names {
            let envelopes: Vec<Envelope> = match run_himalaya_json(&[
                "envelope",
                "list",
                "--folder",
                &folder_name,
                "--account",
                &account_name,
                "--page-size",
                "999",
                "-o",
                "json",
            ]) {
                Ok(envelopes) => envelopes,
                Err(err) => {
                    eprintln!(
                        "warning: failed to fetch messages for account {} folder {}: {:#}",
                        account_name, folder_name, err
                    );
                    continue;
                }
            };

            let envelopes_path = cache_dir
                .join("envelopes")
                .join(&account_name)
                .join(format!("{}.json", &folder_name));
            write_json(&envelopes_path, &envelopes)
                .with_context(|| format!("write {}", envelopes_path.display()))?;

            let progress = ProgressBar::new(envelopes.len() as u64);
            progress.set_style(
                ProgressStyle::with_template(
                    "{spinner:.green} [{elapsed_precise}] {bar:40.cyan/blue} {pos}/{len} {msg}",
                )
                .context("invalid progress bar template")?
                .progress_chars("=>-"),
            );
            progress.set_message(format!("{}/{}", account_name, folder_name));

            let progress = progress.clone();
            let cache_dir = cache_dir.clone();
            let account_name = account_name.clone();
            let folder_name = folder_name.clone();

            envelopes.into_par_iter().for_each(|envelope| {
                let meta_path = cache_dir
                    .join("meta")
                    .join(&account_name)
                    .join(&folder_name)
                    .join(format!("{}.json", &envelope.id));
                if let Err(err) = write_json(&meta_path, &envelope)
                    .with_context(|| format!("write {}", meta_path.display()))
                {
                    eprintln!(
                        "warning: failed to write meta {}: {:#}",
                        meta_path.display(),
                        err
                    );
                    progress.inc(1);
                    return;
                }

                let message_path = cache_dir
                    .join("messages")
                    .join(&account_name)
                    .join(&folder_name)
                    .join(format!("{}.eml", &envelope.id));

                if !message_path.exists() {
                    let message_bytes = match run_himalaya_raw(&[
                        "message",
                        "read",
                        &envelope.id,
                        "--folder",
                        &folder_name,
                        "--account",
                        &account_name,
                    ]) {
                        Ok(message_bytes) => message_bytes,
                        Err(err) => {
                            eprintln!(
                                "warning: failed to read message {} for account {} folder {}: {:#}",
                                envelope.id, account_name, folder_name, err
                            );
                            progress.inc(1);
                            return;
                        }
                    };
                    if let Err(err) = write_bytes(&message_path, &message_bytes)
                        .with_context(|| format!("write {}", message_path.display()))
                    {
                        eprintln!(
                            "warning: failed to write message {}: {:#}",
                            message_path.display(),
                            err
                        );
                        progress.inc(1);
                        return;
                    }
                }

                progress.inc(1);
            });

            progress.finish_with_message(format!("{}/{} complete", account_name, folder_name));
        }
    }
    Ok(())
}

/// Determine the cache root directory.
fn cache_dir() -> Result<PathBuf> {
    let base_dirs = BaseDirs::new().context("locate home directory")?;
    Ok(base_dirs
        .home_dir()
        .join(".local")
        .join("share")
        .join("himalaya-cache"))
}

fn himalaya_path() -> Result<PathBuf> {
    let base_dirs = BaseDirs::new().context("locate home directory")?;
    Ok(base_dirs
        .home_dir()
        .join(".cargo")
        .join("bin")
        .join("himalaya"))
}

/// Write JSON to disk, creating parent directories as needed.
fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create directory {}", parent.display()))?;
    }
    let payload = serde_json::to_vec_pretty(value).context("serialize json")?;
    fs::write(path, payload).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

/// Write raw bytes to disk, creating parent directories as needed.
fn write_bytes(path: &Path, payload: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create directory {}", parent.display()))?;
    }
    fs::write(path, payload).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

/// Run himalaya and deserialize the JSON output, with retry logic.
fn run_himalaya_json<T: for<'de> Deserialize<'de>>(args: &[&str]) -> Result<T> {
    let output = run_himalaya_with_retry(args)?;
    serde_json::from_slice(&output.stdout).context("parse himalaya json")
}

/// Run himalaya and return stdout bytes, with retry logic.
fn run_himalaya_raw(args: &[&str]) -> Result<Vec<u8>> {
    let output = run_himalaya_with_retry(args)?;
    Ok(output.stdout)
}

/// Run a himalaya command and retry up to three attempts on failure.
fn run_himalaya_with_retry(args: &[&str]) -> Result<std::process::Output> {
    let mut last_error = None;
    for attempt in 1..=3 {
        let output = Command::new(himalaya_path()?)
            .args(args)
            .output()
            .with_context(|| format!("run himalaya {}", args.join(" ")))?;
        if output.status.success() {
            return Ok(output);
        }

        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        last_error = Some(stderr);

        if attempt < 3 {
            thread::sleep(Duration::from_millis(2500));
        }
    }

    let stderr = last_error.unwrap_or_else(|| "unknown error".to_string());
    Err(anyhow::anyhow!(
        "himalaya command failed after retries: {}",
        stderr.trim()
    ))
}
