// rmds [--port 3131] [--password <password>] [--latency <ms>] [--jitter <ms>] [--loss <percent>] [--monitor <seconds>]

use std::{
    env,
    net::{Ipv4Addr, Ipv6Addr, SocketAddr},
    process::ExitCode,
    thread,
    time::{Duration, Instant},
};

use net::{Impairment, LossyProxy, PasswordHash, Server, ServerConfig, ServerStats, Traffic, Transport};

const DEFAULT_PORT: u16 = 3131;

fn usage() -> ExitCode {
    log::error!(
        "usage: rmds [--port 3131] [--password <password>] [--latency <ms>] [--jitter <ms>] [--loss <percent>] \
         [--monitor <seconds>]"
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

    let public = SocketAddr::from((Ipv6Addr::UNSPECIFIED, arguments.port));
    let is_simulated = !arguments.impairment.is_none();
    // a simulated link puts the server behind a lossy proxy that takes the public port
    let bind = if is_simulated {
        SocketAddr::from((Ipv4Addr::LOCALHOST, 0))
    } else {
        public
    };
    let server = match Server::spawn(ServerConfig {
        bind,
        codebase: None,
        password: arguments.password.unwrap_or_else(|| {
            let password = net::random_password();
            log::info!("generated password {password}");
            net::hash_password(&password)
        }),
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
    let proxy = match is_simulated
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
    log::info!("listening on {addr}");
    if is_simulated {
        log::info!(
            "simulating {}ms latency, {}ms jitter and {}% loss each way",
            impairment.latency.as_millis(),
            impairment.jitter.as_millis(),
            impairment.loss * 100.0
        );
    }

    let Some(interval) = arguments.monitor else {
        server.wait();

        return ExitCode::SUCCESS;
    };

    let mut previous = server.stats();
    let mut last = Instant::now();
    loop {
        thread::sleep(interval);
        let current = server.stats();
        let now = Instant::now();
        for line in report(&previous, &current, now - last) {
            log::info!("{line}");
        }

        previous = current;
        last = now;
    }
}

fn report(previous: &ServerStats, current: &ServerStats, elapsed: Duration) -> Vec<String> {
    if current.peers.is_empty() {
        return Vec::new();
    }

    let secs = elapsed.as_secs_f64();
    let rate = |transport: Transport| {
        format!(
            "{:.1} kbps {:.0} pkt/s",
            transport.bytes as f64 * 8.0 / 1000.0 / secs,
            transport.packets as f64 / secs
        )
    };
    let since = |now: Transport, before: Option<Transport>| {
        let before = before.unwrap_or_default();
        Transport {
            bytes: now.bytes.saturating_sub(before.bytes),
            packets: now.packets.saturating_sub(before.packets),
        }
    };

    let width = current
        .peers
        .iter()
        .map(|peer| peer.nick.chars().count())
        .max()
        .unwrap_or(0);
    let mut total_sent = Transport::default();
    let mut total_received = Transport::default();
    let mut total_lost = 0;
    let mut peers = Vec::new();
    for peer in &current.peers {
        let before = previous.peers.iter().find(|before| before.id == peer.id);
        let sent = since(peer.sent, before.map(|before| before.sent));
        let received = since(peer.received, before.map(|before| before.received));
        let lost = peer
            .lost_packets
            .saturating_sub(before.map_or(0, |before| before.lost_packets));
        total_sent.bytes += sent.bytes;
        total_sent.packets += sent.packets;
        total_received.bytes += received.bytes;
        total_received.packets += received.packets;
        total_lost += lost;
        peers.push(format!(
            "  {:width$} #{} {} | rtt {}ms | out {} | in {} | lost {lost}",
            peer.nick,
            peer.id.0,
            peer.addr,
            peer.rtt.as_millis(),
            rate(sent),
            rate(received),
        ));
    }

    let mut lines = vec![format!(
        "{} peers | out {} | in {} | lost {total_lost}",
        current.peers.len(),
        rate(total_sent),
        rate(total_received)
    )];
    lines.extend(peers);
    lines.extend(kinds("in ", &previous.received, &current.received));
    lines.extend(kinds("out", &previous.sent, &current.sent));

    lines
}

fn kinds(label: &str, previous: &Traffic, current: &Traffic) -> Option<String> {
    let kinds = current
        .0
        .iter()
        .filter_map(|(kind, count)| {
            let before = previous.0.get(kind).copied().unwrap_or_default();
            let messages = count.messages - before.messages;
            let bytes = count.bytes - before.bytes;
            match (messages, bytes) {
                (0, _) => None,
                (_, 0) => Some(format!("{kind} {messages}")),
                _ => Some(format!("{kind} {messages} ({})", size(bytes))),
            }
        })
        .collect::<Vec<_>>();

    (!kinds.is_empty()).then(|| format!("  {label}: {}", kinds.join(" ")))
}

fn size(bytes: u64) -> String {
    match bytes {
        0..1_000 => format!("{bytes} B"),
        1_000..1_000_000 => format!("{:.1} KB", bytes as f64 / 1_000.0),
        _ => format!("{:.1} MB", bytes as f64 / 1_000_000.0),
    }
}

#[derive(Debug, PartialEq)]
struct Arguments {
    port: u16,
    password: Option<PasswordHash>,
    impairment: Impairment,
    monitor: Option<Duration>,
}

fn parse_arguments(arguments: &[String]) -> Result<Arguments, String> {
    let mut parsed = Arguments {
        port: DEFAULT_PORT,
        password: None,
        impairment: Impairment::default(),
        monitor: None,
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
            "--password" if !value.is_empty() => parsed.password = Some(net::hash_password(value)),
            "--password" => return Err(String::from("the password cannot be empty")),
            "--latency" => parsed.impairment.latency = millis(value)?,
            "--jitter" => parsed.impairment.jitter = millis(value)?,
            "--loss" => match value.parse::<f64>() {
                Ok(percent) if (0.0..=100.0).contains(&percent) => parsed.impairment.loss = percent / 100.0,
                _ => return Err(format!("invalid loss '{value}', expected a percentage")),
            },
            "--monitor" => match value
                .parse()
                .ok()
                .and_then(|secs| Duration::try_from_secs_f64(secs).ok())
            {
                Some(interval) if !interval.is_zero() => parsed.monitor = Some(interval),
                _ => return Err(format!("invalid interval '{value}', expected seconds")),
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
    use net::{PeerId, PeerStats};

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
                monitor: None,
            })
        );
        assert_eq!(
            parse(&["--password", "hunter2", "--port", "9000"]),
            Ok(Arguments {
                port: 9000,
                password: Some(net::hash_password("hunter2")),
                impairment: Impairment::default(),
                monitor: None,
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
        assert_eq!(
            parse(&["--monitor", "2"]).map(|arguments| arguments.monitor),
            Ok(Some(Duration::from_secs(2)))
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
        assert!(parse(&["--monitor", "0"]).is_err());
        assert!(parse(&["--monitor", "x"]).is_err());
    }

    #[test]
    fn reports_rates_since_the_last_snapshot() {
        let peer = |sent, received, lost_packets| PeerStats {
            id: PeerId(1),
            nick: String::from("alice"),
            addr: SocketAddr::from((Ipv4Addr::LOCALHOST, 5000)),
            rtt: Duration::from_millis(18),
            sent: Transport {
                bytes: sent,
                packets: sent / 100,
            },
            received: Transport {
                bytes: received,
                packets: received / 100,
            },
            lost_packets,
        };
        let traffic = |cursors, edits| {
            let mut traffic = Traffic::default();
            for _ in 0..cursors {
                traffic.record("cursor", 50);
            }

            for _ in 0..edits {
                traffic.record("edit", 0);
            }

            traffic
        };

        let previous = ServerStats {
            peers: vec![peer(1_000, 500, 1)],
            received: traffic(2, 1),
            sent: traffic(2, 1),
        };
        let current = ServerStats {
            peers: vec![peer(3_000, 1_500, 2)],
            received: traffic(42, 1),
            sent: traffic(42, 3),
        };

        assert_eq!(
            report(&previous, &current, Duration::from_secs(2)),
            [
                "1 peers | out 8.0 kbps 10 pkt/s | in 4.0 kbps 5 pkt/s | lost 1",
                "  alice #1 127.0.0.1:5000 | rtt 18ms | out 8.0 kbps 10 pkt/s | in 4.0 kbps 5 pkt/s | lost 1",
                "  in : cursor 40 (2.0 KB)",
                "  out: cursor 40 (2.0 KB) edit 2",
            ]
        );
        assert!(report(&previous, &ServerStats::default(), Duration::from_secs(2)).is_empty());
    }
}
