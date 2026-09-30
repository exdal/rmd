// rmds [--port 3131] [--password <password>] [--latency <ms>] [--jitter <ms>] [--loss <percent>]

use std::{
    env,
    net::{Ipv4Addr, SocketAddr},
    process::ExitCode,
    time::Duration,
};

use net::{Impairment, LossyProxy, Server, ServerConfig};

const DEFAULT_PORT: u16 = 3131;

fn usage() -> ExitCode {
    log::error!(
        "usage: rmds [--port 3131] [--password <password>] [--latency <ms>] [--jitter <ms>] [--loss <percent>]"
    );

    ExitCode::FAILURE
}

fn main() -> ExitCode {
    let _ = log::set_logger(&StderrLogger);
    log::set_max_level(log::LevelFilter::Info);

    let arguments = env::args().skip(1).collect::<Vec<_>>();
    let arguments = match parse_arguments(&arguments) {
        Ok(arguments) => arguments,
        Err(error) => {
            log::error!("{error}");

            return usage();
        },
    };

    let public = SocketAddr::from((Ipv4Addr::UNSPECIFIED, arguments.port));
    let simulated = !arguments.impairment.is_none();
    // a simulated link puts the server behind a lossy proxy that takes the public port
    let bind = if simulated {
        SocketAddr::from((Ipv4Addr::LOCALHOST, 0))
    } else {
        public
    };
    let server = match Server::spawn(ServerConfig {
        bind,
        password: arguments.password.unwrap_or_else(net::random_password),
    }) {
        Ok(server) => server,
        Err(e) => {
            log::error!("{e}");

            return ExitCode::FAILURE;
        },
    };

    let impairment = Impairment {
        seed: fastrand::u64(..),
        ..arguments.impairment
    };
    let proxy = match simulated
        .then(|| LossyProxy::spawn(public, server.local_addr(), impairment))
        .transpose()
    {
        Ok(proxy) => proxy,
        Err(e) => {
            log::error!("{e}");

            return ExitCode::FAILURE;
        },
    };

    let addr = proxy.as_ref().map_or(server.local_addr(), LossyProxy::local_addr);
    log::info!("listening on {addr} with password {}", server.password());
    if simulated {
        log::info!(
            "simulating {}ms latency, {}ms jitter and {}% loss each way",
            impairment.latency.as_millis(),
            impairment.jitter.as_millis(),
            impairment.loss * 100.0
        );
    }

    server.wait();

    ExitCode::SUCCESS
}

#[derive(Debug, PartialEq)]
struct Arguments {
    port: u16,
    password: Option<String>,
    impairment: Impairment,
}

fn parse_arguments(arguments: &[String]) -> Result<Arguments, String> {
    let mut parsed = Arguments {
        port: DEFAULT_PORT,
        password: None,
        impairment: Impairment::default(),
    };
    let millis = |value: &str| {
        value
            .parse()
            .map(Duration::from_millis)
            .map_err(|_| format!("invalid milliseconds '{value}'"))
    };

    let mut arguments = arguments.iter();
    while let Some(flag) = arguments.next() {
        let value = arguments.next().ok_or_else(|| format!("{flag} needs a value"))?;
        match flag.as_str() {
            "--port" => parsed.port = value.parse().map_err(|_| format!("invalid port '{value}'"))?,
            "--password" if !value.is_empty() => parsed.password = Some(value.clone()),
            "--password" => return Err(String::from("the password cannot be empty")),
            "--latency" => parsed.impairment.latency = millis(value)?,
            "--jitter" => parsed.impairment.jitter = millis(value)?,
            "--loss" => match value.parse::<f64>() {
                Ok(percent) if (0.0..=100.0).contains(&percent) => parsed.impairment.loss = percent / 100.0,
                _ => return Err(format!("invalid loss '{value}', expected a percentage")),
            },
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
                password: None,
                impairment: Impairment::default(),
            })
        );
        assert_eq!(
            parse(&["--password", "hunter2", "--port", "9000"]),
            Ok(Arguments {
                port: 9000,
                password: Some(String::from("hunter2")),
                impairment: Impairment::default(),
            })
        );
        assert_eq!(
            parse(&["--latency", "150", "--jitter", "50", "--loss", "5"]).map(|arguments| arguments.impairment),
            Ok(Impairment {
                latency: Duration::from_millis(150),
                jitter: Duration::from_millis(50),
                loss: 0.05,
                seed: 0,
            })
        );
    }

    #[test]
    fn rejects_bad_flags() {
        assert!(parse(&["--port"]).is_err());
        assert!(parse(&["--port", "70000"]).is_err());
        assert!(parse(&["--password", ""]).is_err());
        assert!(parse(&["--verbose", "yes"]).is_err());
        assert!(parse(&["--latency", "-5"]).is_err());
        assert!(parse(&["--loss", "120"]).is_err());
    }
}
