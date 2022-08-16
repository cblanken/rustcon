use clap::Parser;
use rustcon::{Rcon, Args};
use std::{io, process::exit};

fn main() -> io::Result<()> {
    let args = Args::parse();
    println!("Connecting to host at {}:{} ...", args.ip, args.port);

    // Establish connection to RCON server
    let rcon = Rcon::new(&args);
    match rcon {
        Ok(r) => r.run()?,
        Err(_) => {
            eprintln!("Unable to create an RCON session with {}:{}", args.ip, args.port);
            eprintln!("Please confirm the server is running.");
            exit(1);
        }
    }

    Ok(())
}
