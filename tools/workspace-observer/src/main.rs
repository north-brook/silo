use std::process;

fn main() {
    if let Err(error) = workspace_observer::run() {
        eprintln!("workspace-observer: {error}");
        process::exit(1);
    }
}
