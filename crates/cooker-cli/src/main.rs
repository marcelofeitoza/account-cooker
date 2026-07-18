//! Account Cooker command-line entry point.

use std::io;

use clap::Parser;

fn main() -> anyhow::Result<()> {
    account_cooker::init_observability()?;
    let cli = account_cooker::Cli::parse();
    tracing::info!(event = "command_started");
    let result = account_cooker::execute(cli, &mut io::stdout().lock());
    match &result {
        Ok(()) => tracing::info!(event = "command_completed"),
        Err(error) => tracing::error!(event = "command_failed", error = %error),
    }
    result
}
