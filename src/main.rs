use clap::Parser;
use is_online::expand_subnets;
use is_online::resolve_hosts;
use is_online::CheckStrategy;
use is_online::IpProtocol;
use is_online::TcpPortCheck;
use std::cmp::min;
use std::collections::HashSet;
use std::process;
use std::thread::available_parallelism;
use std::thread::sleep;
use std::time::Duration;
use std::{io, io::prelude::*};

#[derive(Parser)]
#[clap(version, about)]
/// Check if a port on one or many hosts is online.
struct Cli {
    /// Hosts to check (may also be passed through stdin instead)
    #[clap()]
    hosts: Vec<String>,

    /// Port to check
    #[clap(short, long, default_value = "22")]
    port: u16,

    /// TCP connection timeout in milliseconds
    #[clap(short, long, default_value = "1000")]
    timeout: u32,

    /// Limit to IPv4
    #[clap(short = '4')]
    ipv4_only: bool,

    /// Limit to IPv6
    #[clap(short = '6')]
    ipv6_only: bool,

    /// Exit with 1 if any of the hosts are offline
    #[clap(short, long)]
    fail: bool,

    /// Do not print anything
    #[clap(short, long)]
    quiet: bool,

    /// Do not print colors
    #[clap(long = "no-color", env = "NO_COLOR")]
    no_color: bool,

    /// Ensure that all addresses the host resolves to are online (by default, only one has to be)
    #[clap(long = "all")]
    all: bool,

    /// Wait for the hosts to be online, then only exit
    #[clap(short, long)]
    wait: bool,

    /// Clear screen before showing results (useful when waiting)
    #[clap(short, long)]
    clear: bool,

    /// The number of threads to used for IO. If zero, the number of cores multiplied by 4 are used.
    #[clap(long, default_value = "0")]
    workers: u8,
}

impl Cli {
    /// The number of threads that should be used
    fn threads(&self) -> usize {
        let parallelism: usize = match available_parallelism() {
            Ok(value) => value.get(),
            _ => 1,
        };

        match self.workers {
            0 => min(parallelism * 4, 255) as usize,
            _ => min(self.workers, 255) as usize,
        }
    }

    /// The protocol to use
    fn protocol(&self) -> IpProtocol {
        match (self.ipv4_only, self.ipv6_only) {
            (true, false) => IpProtocol::V4,
            (false, true) => IpProtocol::V6,
            _ => IpProtocol::Both,
        }
    }

    /// The check strategy
    fn strategy(&self) -> CheckStrategy {
        if self.all {
            CheckStrategy::All
        } else {
            CheckStrategy::Any
        }
    }

    /// The timeout as duration
    fn timeout_duration(&self) -> Duration {
        Duration::from_millis(self.timeout as u64)
    }

    /// The port check to execute
    fn tcp_port_check(&self) -> TcpPortCheck {
        TcpPortCheck::default()
            .with_port(self.port)
            .with_protocol(self.protocol())
            .with_timeout(self.timeout_duration())
            .with_strategy(self.strategy())
    }

    /// Clear the screen
    fn clear_screen() {
        print!("{esc}[2J{esc}[1;1H", esc = 27 as char);
    }

    /// Formats the given text, in color if enabled
    fn format_output(&self, text: &str, style: FormatStyle) -> String {
        match (self.no_color, style) {
            (false, FormatStyle::Success) => format!("\x1b[1;31m{}\x1b[0m", text),
            (false, FormatStyle::Failure) => format!("\x1b[1;32m{}\x1b[0m", text),
            _ => text.to_string(),
        }
    }

    /// Formats the given text as success
    fn format_success(&self, text: &str) -> String {
        self.format_output(text, FormatStyle::Success)
    }

    /// Formats the given text as failure
    fn format_failure(&self, text: &str) -> String {
        self.format_output(text, FormatStyle::Failure)
    }
}

enum FormatStyle {
    Success,
    Failure,
}

fn main() {
    let cli = Cli::parse();

    // Globally configure the number of threads used
    rayon::ThreadPoolBuilder::new()
        .num_threads(cli.threads())
        .build_global()
        .unwrap();

    // Build the port check
    let check = cli.tcp_port_check();

    // Read the hosts from the list, or from stdin
    let hosts = match cli.hosts.is_empty() {
        true => expand_subnets(
            &io::stdin()
                .lock()
                .lines()
                .map(|l| l.unwrap())
                .collect::<Vec<String>>(),
        ),
        false => expand_subnets(&cli.hosts),
    };

    loop {
        // Clear at the beginning of each loop
        if cli.clear {
            Cli::clear_screen();
        }

        // Gather the resolved/online hosts
        let resolved = resolve_hosts(&hosts);
        let online = check.collect_online(&resolved);

        // Keep a set of resolved/online hosts
        let resolved: HashSet<String> = resolved.iter().map(|h| h.name.to_string()).collect();
        let online: HashSet<String> = online.iter().map(|h| h.name.to_string()).collect();

        // Exit with a 1 --fail is given and not all hosts are online
        let exit_code = if cli.fail && online.len() != hosts.len() {
            1
        } else {
            0
        };

        // Note what went wrong
        if !cli.quiet {
            for name in hosts.iter().by_ref() {
                if !resolved.contains(name) {
                    println!("{name} could not be resolved");
                } else if !online.contains(name) {
                    println!("{name}:{} is {}", cli.port, cli.format_failure("offline"));
                } else {
                    println!("{name}:{} is {}", cli.port, cli.format_success("online"));
                }
            }
        }

        // Maybe repeat
        let try_again = cli.wait && hosts.iter().by_ref().any(|h| !online.contains(h));
        if !try_again {
            process::exit(exit_code);
        } else {
            sleep(Duration::from_secs(1));
        }
    }
}
