// rmds [--port 3131] [--password <password>]

use std::{
    net::{Ipv4Addr, SocketAddr},
    process::ExitCode,
};

use net::{Server, ServerConfig};

const DEFAULT_PORT: u16 = 3131;

fn usage() -> ExitCode {
    log::error!("usage: rmds [--port 3131] [--password <password>]");

    ExitCode::FAILURE
}

fn main() -> ExitCode {
    let _ = log::set_logger(&StderrLogger);
    log::set_max_level(log::LevelFilter::Info);

    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    let arguments = match parse_arguments(&arguments) {
        Ok(arguments) => arguments,
        Err(error) => {
            log::error!("{error}");

            return usage();
        },
    };

    let server = match Server::spawn(ServerConfig {
        bind: SocketAddr::from((Ipv4Addr::UNSPECIFIED, arguments.port)),
        password: arguments.password.unwrap_or_else(net::random_password),
    }) {
        Ok(server) => server,
        Err(e) => {
            log::error!("{e}");

            return ExitCode::FAILURE;
        },
    };

    log::info!(
        "listening on {} with password {}",
        server.local_addr(),
        server.password()
    );
    server.wait();

    ExitCode::SUCCESS
}

#[derive(Debug, PartialEq, Eq)]
struct Arguments {
    port: u16,
    password: Option<String>,
}

fn parse_arguments(arguments: &[String]) -> Result<Arguments, String> {
    let mut parsed = Arguments {
        port: DEFAULT_PORT,
        password: None,
    };

    let mut arguments = arguments.iter();
    while let Some(flag) = arguments.next() {
        let value = arguments.next().ok_or_else(|| format!("{flag} needs a value"))?;
        match flag.as_str() {
            "--port" => parsed.port = value.parse().map_err(|_| format!("invalid port '{value}'"))?,
            "--password" if !value.is_empty() => parsed.password = Some(value.clone()),
            "--password" => return Err(String::from("the password cannot be empty")),
            _ => return Err(format!("unknown flag '{flag}'")),
        }
    }

    Ok(parsed)
}

struct StderrLogger;

impl log::Log for StderrLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool { metadata.level() <= log::Level::Info }

    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            eprintln!("[{}] {}", record.level(), record.args());
        }
    }

    fn flush(&self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(arguments: &[&str]) -> Result<Arguments, String> {
        parse_arguments(
            &arguments
                .iter()
                .map(|argument| argument.to_string())
                .collect::<Vec<_>>(),
        )
    }

    #[test]
    fn defaults_and_flags() {
        assert_eq!(
            parse(&[]),
            Ok(Arguments {
                port: DEFAULT_PORT,
                password: None
            })
        );
        assert_eq!(
            parse(&["--password", "hunter2", "--port", "9000"]),
            Ok(Arguments {
                port: 9000,
                password: Some(String::from("hunter2"))
            })
        );
    }

    #[test]
    fn rejects_bad_flags() {
        assert!(parse(&["--port"]).is_err());
        assert!(parse(&["--port", "70000"]).is_err());
        assert!(parse(&["--password", ""]).is_err());
        assert!(parse(&["--verbose", "yes"]).is_err());
    }
}
