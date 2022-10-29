use clap::Parser;
use is_online::expand_subnets;
use is_online::resolve_hosts;
use is_online::CheckStrategy;
use is_online::IpProtocol;
use is_online::TcpPortCheck;
use std::cmp::min;
use std::collections::HashSet;
use std::env;
use std::process;
use std::thread::available_parallelism;
use std::thread::sleep;
use std::time::Duration;
use std::{io, io::prelude::*};

#[derive(Parser)]
#[clap(version, about)]
/// Check if a port on one or many hosts is online.
struct Args {
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
    #[clap(long = "no-color")]
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

impl Args {
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
}

fn main() {
    let args = Args::parse();

    rayon::ThreadPoolBuilder::new()
        .num_threads(args.threads())
        .build_global()
        .unwrap();

    let check = args.tcp_port_check();

    // Read the hosts from the list, or from stdin
    let hosts = match args.hosts.is_empty() {
        true => io::stdin().lock().lines().map(|l| l.unwrap()).collect(),
        false => args.hosts,
    };

    let hosts = expand_subnets(&hosts);

    loop {
        if args.clear {
            print!("{esc}[2J{esc}[1;1H", esc = 27 as char);
        }

        let resolved = resolve_hosts(&hosts);
        let online = check.collect_online(&resolved);
        let resolved: HashSet<String> = resolved.iter().map(|h| h.name.to_string()).collect();
        let online: HashSet<String> = online.iter().map(|h| h.name.to_string()).collect();

        let exit_code = if args.fail && online.len() != hosts.len() {
            1
        } else {
            0
        };

        let show_color = !(args.no_color || env::var("NO_COLOR").is_ok());
        let offline_text = if show_color {
            "\x1b[1;31moffline\x1b[0m"
        } else {
            "offline"
        };
        let online_text = if show_color {
            "\x1b[1;32monline\x1b[0m"
        } else {
            "online"
        };

        if !args.quiet {
            for name in hosts.iter().by_ref() {
                if !resolved.contains(name) {
                    println!("{name} could not be resolved");
                } else if !online.contains(name) {
                    println!("{name}:{} is {offline_text}", args.port);
                } else {
                    println!("{name}:{} is {online_text}", args.port);
                }
            }
        }

        let try_again = args.wait && hosts.iter().by_ref().any(|h| !online.contains(h));
        if !try_again {
            process::exit(exit_code);
        } else {
            sleep(Duration::from_secs(1));
        }
    }
}
