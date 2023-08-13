use clap::Parser;
use rustcon::{Args, Rcon};
use std::{io, process::exit};

fn main() -> io::Result<()> {
    let args = Args::parse();
    println!("Connecting to host at {}:{} ...", args.ip, args.port);

    // Establish connection to RCON server
    loop {
        match Rcon::new(&args) {
            // start default rcon shell
            Ok(r) => {
                if let Err(_) = r.run() {
                    eprintln!("Lost connection to RCON server!");
                    eprintln!("Attempting to reconnect...");
                    continue;
                }
            }
            Err(_) => {
                eprintln!(
                    "Unable to create an RCON session to {}:{}",
                    args.ip, args.port
                );
                eprintln!("Please confirm the server is running.");
                let stdin = io::stdin();
                let mut buffer = String::new();
                loop {
                    eprint!("Try again? (y/n): ");
                    stdin.read_line(&mut buffer)?;
                    match buffer.trim() {
                        "y" | "yes" | "Y" | "YES" => {
                            buffer.clear();
                            break;
                        }
                        "n" | "no" | "N" | "NO" => exit(1),
                        _ => {
                            buffer.clear();
                            continue;
                        }
                    }
                }
            }
        };
    }
}
