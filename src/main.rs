extern crate clap;
extern crate rpassword;
extern crate rustcon;

use clap::{Parser};
use rustcon::{Rcon};
use std::{io};

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    /// RCON server IPv4 address
    #[clap(short, long, default_value = "127.0.0.1")]
    host: String,

    /// RCON server PORT number
    #[clap(short, long, default_value = "27015")]
    port: String,
}

fn main() -> io::Result<()> {
    let args = Args::parse();
    println!("Connecting to host at {}:{} ...", args.host, args.port);

    // Establish connection to RCON server
    let pass = rpassword::read_password_from_tty(Some("Password: ")).unwrap();
    let mut rcon = Rcon::new(&args.host, &args.port).unwrap();
    
    // Authenticate with RCON password
    let auth = rcon.authenticate(&pass).unwrap();
    println!("{}", auth);
    
    // Send test "help" command
    let help = rcon.send_cmd("help").unwrap();
    println!("{}", help);
    Ok(())
}
