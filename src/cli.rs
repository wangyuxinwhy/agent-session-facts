use std::io::Write as _;
use std::path::PathBuf;

use clap::Args;
use clap::Parser;
use clap::Subcommand;
use clap::ValueEnum;
use clap::error::ErrorKind;

use crate::claude;
use crate::codex;
use crate::error::AppError;
use crate::pi;

#[derive(Debug, Parser)]
#[command(
    name = "agent-session-facts",
    version,
    about = "Extract auditable facts from native Agent runtime sessions",
    long_about = "Extract auditable facts from native Agent runtime session artifacts.\n\nThis binary reads artifacts directly. It does not depend on RunLab or Trace Index and does not judge task quality.",
    after_long_help = "WORKFLOW:\n  agent-session-facts session inspect --runtime <codex|pi|claude> ARTIFACT...\n\nRun 'agent-session-facts session inspect --help' for the accepted input, JSON output, completeness, and exit-status contracts."
)]
struct Cli {
    #[command(subcommand)]
    entity: Entity,
}

#[derive(Debug, Subcommand)]
enum Entity {
    /// Inspect runtime-maintained Agent session artifacts.
    #[command(
        long_about = "Inspect runtime-maintained Agent session artifacts.\n\nUse 'session inspect' to turn one native Session's rollout files into an auditable JSON fact report."
    )]
    Session(SessionCommand),
}

#[derive(Debug, Args)]
struct SessionCommand {
    #[command(subcommand)]
    action: SessionAction,
}

#[derive(Debug, Subcommand)]
enum SessionAction {
    /// Emit one JSON fact report from native Session artifacts.
    #[command(
        long_about = "Read one native Agent Session's artifacts and emit one compact JSON fact report.\n\nThe uniform view uses the shared Session, Loop, Item, Source, and normalized usage vocabulary. The runtime-specific view preserves facts that cannot be normalized without losing meaning.",
        after_long_help = "INPUT:\n  Codex   1 to 1024 on-disk rollout JSONL files with one native session_id.\n          Supply child Thread rollouts when they are part of the Session.\n          The distinct 'codex exec --json' event stream is not accepted.\n  Pi      Exactly one native Session JSONL file. Resume appends to that file; a fork is another Session.\n  Claude  1 to 1024 JSONL files with one sessionId, including exactly one root source.\n          Supply runtime-declared subagent sources for complete coverage.\n\nOUTPUT:\n  stdout  One compact agent_session_facts.report JSON object followed by a newline.\n  stderr  One compact agent_session_facts.error JSON object on command failure.\n\nCOMPLETENESS:\n  Exit 0 means a report was produced. Inspect its 'complete' field: false means the supplied artifacts were readable but incomplete, and 'incomplete_reasons' explains why.\n\nEXIT STATUS:\n  0  Report produced, including a report with complete=false.\n  1  I/O or internal failure.\n  2  Invalid or unsupported input.\n\nEXAMPLES:\n  agent-session-facts session inspect --runtime codex root.jsonl child.jsonl\n  agent-session-facts session inspect --runtime pi session.jsonl\n  agent-session-facts session inspect --runtime claude session.jsonl subagents/*.jsonl"
    )]
    Inspect(InspectArgs),
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum Runtime {
    Claude,
    Codex,
    Pi,
}

#[derive(Debug, Args)]
struct InspectArgs {
    /// Runtime that owns the artifact format.
    #[arg(long, value_enum)]
    runtime: Runtime,

    /// On-disk JSONL artifacts from one native Session.
    #[arg(value_name = "ARTIFACT", required = true, num_args = 1..=1024)]
    artifacts: Vec<PathBuf>,
}

pub(crate) fn run() -> Result<i32, AppError> {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => return handle_parse_error(&error),
    };
    match cli.entity {
        Entity::Session(SessionCommand {
            action: SessionAction::Inspect(args),
        }) => {
            let report = match args.runtime {
                Runtime::Claude => claude::inspect(&args.artifacts)?,
                Runtime::Codex => codex::inspect(&args.artifacts)?,
                Runtime::Pi => pi::inspect(&args.artifacts)?,
            };
            serde_json::to_writer(std::io::stdout().lock(), &report)
                .map_err(|error| AppError::internal(format!("failed to encode report: {error}")))?;
            println!();
            Ok(0)
        }
    }
}

fn handle_parse_error(error: &clap::Error) -> Result<i32, AppError> {
    match error.kind() {
        ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => {
            let mut stdout = std::io::stdout().lock();
            stdout
                .write_all(error.to_string().as_bytes())
                .map_err(|write_error| {
                    AppError::io(format!("failed to write command help: {write_error}"))
                })?;
            Ok(0)
        }
        _ => Err(AppError::invalid(error.to_string().trim_end().to_owned())),
    }
}
