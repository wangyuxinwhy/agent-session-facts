#![forbid(unsafe_code)]

mod claude;
mod cli;
mod codex;
mod error;
mod model;
mod pi;

fn main() {
    let exit = match cli::run() {
        Ok(code) => code,
        Err(error) => {
            error::emit(&error);
            error.exit_code()
        }
    };
    std::process::exit(exit);
}
