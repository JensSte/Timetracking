//! Command-line tool for the orchestrator to file time into the app (ADR-0041).
//! Run `timetracking-agent --help` for usage.

fn main() {
    std::process::exit(timetracking_lib::run_agent_cli());
}
