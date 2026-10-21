use std::env;
use std::process::ExitCode;

use uzda_agent_claude::ClaudeAgent;
use uzda_agent_codex::CodexAgent;
use uzda_core::{AgentRuntime, RuntimeAvailability};

fn main() -> ExitCode {
    match env::args().nth(1).as_deref() {
        None => {
            print_help();
            ExitCode::SUCCESS
        }
        Some("agents") => {
            print_agents();
            ExitCode::SUCCESS
        }
        Some("help" | "--help" | "-h") => {
            print_help();
            ExitCode::SUCCESS
        }
        Some(command) => {
            eprintln!("Unknown command: {command}\n");
            print_help();
            ExitCode::from(2)
        }
    }
}

fn print_help() {
    println!("{}", uzda_core::welcome_message());
    println!();
    println!("Usage:");
    println!("    uzda agents    Show local agent runtimes");
    println!("    uzda help      Show this help");
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
