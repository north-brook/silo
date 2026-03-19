pub(crate) mod args;
pub(crate) mod assistant;
pub(crate) mod cli;
pub(crate) mod daemon;
pub(crate) mod files;
pub(crate) mod metadata;
pub(crate) mod runtime;

#[cfg(test)]
mod tests;

pub fn run() -> Result<(), String> {
    cli::run()
}
