use std::process;

fn main() {
    if let Err(error) = workspace_agent::run() {
        eprintln!("workspace-agent: {error}");
        process::exit(1);
    }
}
