use std::path::PathBuf;
use std::process::ExitCode;

use vcmp_protocol::Role;
use vcmp_server::{Config, Server};

const USAGE: &str = "\
Usage: vcmp-server [--config <file>] [command]

Commands:
  serve                 Serve the library (the default)
  tokens                Show the server's id and the tokens the app connects with
  new-token user|admin  Replace a token; a running server takes it within seconds

Options:
  --config <file>       A TOML configuration file (or VCMP_SERVER_CONFIG)
  -h, --help            Show this help
  -V, --version         Show the version";

fn main() -> ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let mut args = std::env::args().skip(1);
    let mut config_file = None;
    let mut words = Vec::new();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--config" => match args.next() {
                Some(path) => config_file = Some(PathBuf::from(path)),
                None => return usage_error("--config needs a file"),
            },
            "-h" | "--help" => {
                println!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            "-V" | "--version" => {
                println!("vcmp-server {}", env!("CARGO_PKG_VERSION"));
                return ExitCode::SUCCESS;
            }
            _ => words.push(arg),
        }
    }
    let config = match Config::load(config_file.as_deref(), |key| std::env::var(key).ok()) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("vcmp-server: {error}");
            return ExitCode::FAILURE;
        }
    };
    match words
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        [] | ["serve"] => serve(config),
        ["tokens"] => show_tokens(&config),
        ["new-token", role] => match Role::parse(role) {
            Some(role) => new_token(&config, role),
            None => usage_error("new-token takes user or admin"),
        },
        _ => usage_error("unknown command"),
    }
}

fn usage_error(message: &str) -> ExitCode {
    eprintln!("vcmp-server: {message}\n\n{USAGE}");
    ExitCode::from(2)
}

fn serve(config: Config) -> ExitCode {
    log::info!(
        "VCMP server {} with its data in {}",
        env!("CARGO_PKG_VERSION"),
        config.data.display()
    );
    let server = match Server::start(config) {
        Ok(server) => server,
        Err(error) => {
            log::error!("{error}");
            return ExitCode::FAILURE;
        }
    };
    log::info!("listening on http://{}", server.address());
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            log::error!("waiting for a stop signal failed: {error}");
            return ExitCode::FAILURE;
        }
    };
    runtime.block_on(stop_signal());
    log::info!("stopping");
    drop(server);
    ExitCode::SUCCESS
}

async fn stop_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut terminate) => {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    _ = terminate.recv() => {}
                }
            }
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

fn show_tokens(config: &Config) -> ExitCode {
    match vcmp_server::tokens(config) {
        Ok((server_id, tokens)) => {
            println!("Server id:   {server_id}");
            for token in tokens {
                let label = match token.role {
                    Role::User => "User token: ",
                    Role::Admin => "Admin token:",
                };
                let source = if token.configured {
                    " (set by the configuration)"
                } else {
                    ""
                };
                println!("{label} {}{source}", token.token);
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("vcmp-server: {error}");
            ExitCode::FAILURE
        }
    }
}

fn new_token(config: &Config, role: Role) -> ExitCode {
    match vcmp_server::replace_token(config, role) {
        Ok(token) => {
            println!("{token}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("vcmp-server: {error}");
            ExitCode::FAILURE
        }
    }
}
