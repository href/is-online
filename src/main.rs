mod host;

use crate::host::TcpPortCheck;
use clap::Parser;
use host::expand_subnets;
use host::resolve_hosts;
use host::CheckStrategy;
use host::IpProtocol;
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
}

fn main() {
    let args = Args::parse();
    let timeout = Duration::new(0, args.timeout * 1_000_000);

    rayon::ThreadPoolBuilder::new()
        .num_threads(args.threads())
        .build_global()
        .unwrap();

    // Pick the protocol to use
    let protocol = match (args.ipv4_only, args.ipv6_only) {
        (true, false) => IpProtocol::V4,
        (false, true) => IpProtocol::V6,
        _ => IpProtocol::Both,
    };

    // Read the hosts from the list, or from stdin
    let hosts = match args.hosts.is_empty() {
        true => io::stdin().lock().lines().map(|l| l.unwrap()).collect(),
        false => args.hosts,
    };

    let strategy = if args.all {
        CheckStrategy::All
    } else {
        CheckStrategy::Any
    };

    let check = TcpPortCheck::default()
        .with_port(args.port)
        .with_protocol(protocol)
        .with_timeout(timeout)
        .with_strategy(strategy);

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
