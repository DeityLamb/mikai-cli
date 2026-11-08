mod api;
mod cli;
mod config;
mod models;
mod style;
mod tui;

use std::process::ExitCode;

use clap::Parser;

use crate::{
    cli::{Cli, Command, Ctx},
    config::Config,
};

fn main() -> ExitCode {
    // Die quietly on a closed pipe (`mikai … | head`) instead of panicking.
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
    let args = Cli::parse();
    style::init(args.no_color);
    match run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{} {e:#}", style::red("помилка:"));
            ExitCode::FAILURE
        }
    }
}

fn run(args: Cli) -> anyhow::Result<()> {
    let config = Config::load()?;
    let key = args.key.clone().or_else(|| config.key.clone());
    let client = api::Client::new(key)?;
    match args.command {
        None | Some(Command::Tui) => {
            let user = config.user.clone().or_else(|| config.key.as_ref().map(|_| "увійшли".into()));
            tui::run(client, user)
        }
        Some(cmd) => {
            let mut ctx = Ctx { client, config, json: args.json };
            cli::run(cmd, &mut ctx)
        }
    }
}
