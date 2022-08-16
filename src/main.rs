use clap::Parser;
use rustcon::{Rcon, Args};
use std::io;

fn main() -> io::Result<()> {
    let args = Args::parse();
    println!("Connecting to host at {}:{} ...", args.ip, args.port);

    // Establish connection to RCON server
    let rcon = Rcon::new(&args)
        .expect(format!("Unable to create an RCON session with {}:{}", args.ip, args.port).as_str());
    rcon.run()?;

    Ok(())
}
