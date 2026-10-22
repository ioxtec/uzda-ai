use std::env;
use std::error::Error;
use std::io::{self, Write};
use std::process::ExitCode;

use uzda_agent_claude::ClaudeAgent;
use uzda_agent_codex::{CodexAgent, CodexAppServer, CodexError, CodexThreadOptions};
use uzda_core::{
    AgentEvent, AgentKind, AgentRuntime, AgentSession, ApprovalDecision, LocalSessionStore,
    RuntimeAvailability,
};

type CliResult<T = ()> = Result<T, Box<dyn Error>>;

fn main() -> ExitCode {
    let arguments: Vec<String> = env::args().skip(1).collect();

    match arguments.as_slice() {
        [] => {
            print_help();
            ExitCode::SUCCESS
        }
        [command] if command == "agents" => {
            print_agents();
            ExitCode::SUCCESS
        }
        [command] if matches!(command.as_str(), "help" | "--help" | "-h") => {
            print_help();
            ExitCode::SUCCESS
        }
        [command, action] if command == "codex" && action == "status" => {
            match print_codex_status() {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    eprintln!("Could not connect to Codex: {error}");
                    ExitCode::FAILURE
                }
            }
        }
        [command, action] if command == "codex" && action == "start" => {
            match start_codex_thread() {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    eprintln!("Could not start a Codex thread: {error}");
                    ExitCode::FAILURE
                }
            }
        }
        [command, action, prompt @ ..] if command == "codex" && action == "ask" => {
            match ask_codex(&prompt.join(" ")) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    eprintln!("Could not complete the Codex turn: {error}");
                    ExitCode::FAILURE
                }
            }
        }
        _ => {
            eprintln!("Unknown command: {}\n", arguments.join(" "));
            print_help();
            ExitCode::from(2)
        }
    }
}

fn print_help() {
    println!("{}", uzda_core::welcome_message());
    println!();
    println!("Usage:");
    println!("    uzda agents          Show local agent runtimes");
    println!("    uzda codex status    Test the Codex app-server connection");
    println!("    uzda codex start     Start a Codex thread in this directory");
    println!("    uzda codex ask TEXT  Ask Codex and stream its answer");
    println!("    uzda help            Show this help");
}

fn print_agents() {
    let codex = CodexAgent::default();
    let claude = ClaudeAgent::default();
    let agents: [&dyn AgentRuntime; 2] = [&codex, &claude];

    println!("Local agent runtimes:");

    for agent in agents {
        let descriptor = agent.descriptor();
        let availability = format_availability(agent.availability());

        println!("    {:<12} {availability}", descriptor.display_name);
    }
}

fn print_codex_status() -> Result<(), CodexError> {
    let agent = CodexAgent::default();
    let server = agent.connect()?;
    let info = server.server_info();

    println!("Connected to Codex app-server");
    println!("    Runtime:  {}", info.user_agent);
    println!(
        "    Platform: {} ({})",
        info.platform_os, info.platform_family
    );

    Ok(())
}

fn start_codex_thread() -> CliResult {
    let agent = CodexAgent::default();
    let mut server = agent.connect()?;
    let project_directory = env::current_dir()?;
    let options = CodexThreadOptions::new(&project_directory);
    let session = server.start_thread(options)?;
    let store = LocalSessionStore::for_project(project_directory);
    store.save(&session)?;

    println!("Started Codex thread");
    println!("    Session ID: {}", session.id.as_str());
    println!("    Saved in:   {}", store.path().display());

    Ok(())
}

fn ask_codex(prompt: &str) -> CliResult {
    let agent = CodexAgent::default();
    let mut server = agent.connect()?;
    let project_directory = env::current_dir()?;
    let store = LocalSessionStore::for_project(&project_directory);
    let session = open_codex_session(&mut server, &store, &project_directory)?;

    server.run_turn(
        &session,
        prompt,
        |event| match event {
            AgentEvent::TextDelta(text) => {
                print!("{text}");
                let _ = io::stdout().flush();
            }
            AgentEvent::ToolStarted { name } => eprintln!("[tool started] {name}"),
            AgentEvent::ToolCompleted { name, succeeded } => {
                let status = if succeeded { "completed" } else { "failed" };
                eprintln!("[tool {status}] {name}");
            }
            AgentEvent::ApprovalRequired { description } => {
                eprintln!("\nApproval required:\n{description}");
            }
            AgentEvent::TurnCompleted => println!(),
            AgentEvent::TurnFailed { .. } => {}
        },
        ask_for_approval,
    )?;

    Ok(())
}

fn open_codex_session(
    server: &mut CodexAppServer,
    store: &LocalSessionStore,
    project_directory: &std::path::Path,
) -> CliResult<AgentSession> {
    if let Some(saved) = store.load(AgentKind::Codex)? {
        return Ok(server.resume_thread(&saved.id)?);
    }

    let session = server.start_thread(CodexThreadOptions::new(project_directory))?;
    store.save(&session)?;

    Ok(session)
}

fn ask_for_approval(_description: &str) -> ApprovalDecision {
    eprint!("Approve once? [y/N] ");
    let _ = io::stderr().flush();

    let mut answer = String::new();
    match io::stdin().read_line(&mut answer) {
        Ok(_) if matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") => {
            ApprovalDecision::Approve
        }
        _ => ApprovalDecision::Decline,
    }
}

fn format_availability(availability: RuntimeAvailability) -> String {
    match availability {
        RuntimeAvailability::Available {
            version: Some(version),
        } => format!("available ({version})"),
        RuntimeAvailability::Available { version: None } => "available".to_owned(),
        RuntimeAvailability::NotFound => "not found".to_owned(),
        RuntimeAvailability::Unavailable { reason } => format!("unavailable ({reason})"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_available_runtime_with_version() {
        let availability = RuntimeAvailability::Available {
            version: Some("1.0.0".to_owned()),
        };

        assert_eq!(format_availability(availability), "available (1.0.0)");
    }

    #[test]
    fn formats_missing_runtime() {
        assert_eq!(
            format_availability(RuntimeAvailability::NotFound),
            "not found"
        );
    }
}
