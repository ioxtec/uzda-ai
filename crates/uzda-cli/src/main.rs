use std::env;
use std::process::ExitCode;

use uzda_agent_claude::ClaudeAgent;
use uzda_agent_codex::{CodexAgent, CodexError};
use uzda_core::{AgentRuntime, RuntimeAvailability};

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
