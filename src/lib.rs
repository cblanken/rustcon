/*
 * An interactive RCON shell.
 */

extern crate bytes;
extern crate clap;
extern crate rpassword;

use bytes::{Bytes, BytesMut, Buf, BufMut};
use clap::{Parser};
use std::{
    io::{self, Write, Read},
    fmt,
    net::{TcpStream},
    str,
    time::{Duration},
};

// TODO: add verbose parameter
#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
pub struct Args {
    /// RCON server IPv4 address
    #[clap(short, long, default_value = "127.0.0.1")]
    pub ip: String,

    /// RCON server PORT number
    #[clap(short, long, default_value = "27015")]
    pub port: String,
}

#[derive(Clone)]
pub enum PacketType {
    Login = 3,      // SERVERDATA_AUTH
    Command = 2,    // SERVERDATA_EXECCOMMAND or SERVERDATA_AUTH_RESPONSE
    Response = 0,   // SERVERDATA_RESPONSE_VALUE
    Unknown,
}

impl From<i32> for PacketType {
    fn from(num: i32) -> Self {
        match num {
            3 => PacketType::Login,
            2 => PacketType::Command,
            0 => PacketType::Response,
            _ => PacketType::Unknown,
        }
    }
}

impl fmt::Display for PacketType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            PacketType::Login => write!(f, "Login"),
            PacketType::Command => write!(f, "Command/Auth Response"),
            PacketType::Response => write!(f, "Response Data"),
            _ => write!(f, "UNKNOWN"),
        }
    }
}

const MAX_PACKET_SIZE: usize = 4096;

/// RCON packet struct
pub struct Packet {
    /// Length of remainder of packet, max of 4096 for a single packet
    size: i32,

    /// Client-generated ID
    id: i32,

    /// 3 for login: SERVERDATA_AUTH
    /// 2 for auth response or run a command: SERVERDATA_AUTH_RESPONSE or SERVERDATA_EXECCOMMAND
    /// 0 for multi-packet response: SERVERDATA_RESPONSE_VALUE
    typ: PacketType,

    /// Body
    body_text: String,
    body_bytes: Bytes,

    /// 1-byte pad / empty byte
    pad: u8,
}

#[derive(Debug)]
pub enum PacketError {
    InvalidSize,
    LargePacket,
    NonAscii,
}

type PacketResult = Result<Packet, PacketError>;

impl Packet {
    /// Initialize a packet instance with calculated length and included pad byte
    pub fn new(id: i32, typ: PacketType, body_text: String) -> PacketResult {
        let body_bytes = Bytes::from(body_text.clone());
        if !body_bytes.is_ascii() {
            Err(PacketError::NonAscii)
        } else if body_bytes.len() + 10 > MAX_PACKET_SIZE { // packets larger then 4096 bytes should be split
            Err(PacketError::LargePacket)
        } else {
            let packet = Packet {
                size: body_bytes.len() as i32 + 10,
                id: id,
                typ: typ,
                body_text: body_text,
                body_bytes: body_bytes,
                pad: 0,
            };
            
            Ok(packet)
        }
    }

    fn replace_color_codes(s: String) -> String {
        // Replace any color codes and non-ascii chars
        s
            .replace("§6", "")
            .replace("§7", "")
            .replace("§e", "")
            .replace("§f", "")
    }

    fn deserialize(mut bytes: Bytes) -> PacketResult {
        let size = bytes.get_i32_le();
        if size < 10 || size > MAX_PACKET_SIZE as i32 {
            Err(PacketError::InvalidSize)?
        }
        let id = bytes.get_i32_le();
        let typ = PacketType::from(bytes.get_i32_le());
        let body_bytes = bytes.copy_to_bytes(size as usize - 9);

        if body_bytes.len() + 10 > MAX_PACKET_SIZE { // packets larger then 4096 bytes should be split
            Err(PacketError::LargePacket)?
        }

        let packet = Packet {
            size: size,
            id: id,
            typ: typ,
            body_text: Packet::replace_color_codes(str::from_utf8(&body_bytes).unwrap().to_string()),
            body_bytes: body_bytes,
            pad: 0,
        };
        Ok(packet)
    }

    /// Serialize packet into a Vec<u8>
    fn serialize(&self) -> BytesMut {
        let mut p = BytesMut::with_capacity(MAX_PACKET_SIZE);

        // Construct packet data in bytes
        p.put_i32_le(self.size);
        p.put_i32_le(self.id);
        p.put_i32_le(self.typ.clone() as i32);
        p.put(self.body_bytes.clone());
        p.put_u8('\0' as u8);   // terminate body with null byte
        p.put_u8(self.pad);     // append pad null byte
        return p;
    }
}

impl fmt::Display for Packet {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Size: {} bytes, ID: {}, Type: {}\n{}", self.size, self.id, self.typ, self.body_text)
    }
}

pub fn send_packet(packet: &Packet, mut stream: &TcpStream) -> io::Result<()> {
    let mut packet_bytes = packet.serialize();
    println!("<<< Sending packet: {}", packet);
    //println!("Bytes: {:#x}", packet_bytes);
    stream.write(packet_bytes.as_mut()).expect("Cannot write data to stream.");
    //stream.flush()?;
    Ok(())
}

/// RCON connection struct for handling sending and receiving RCON packets
pub struct Rcon {
    // Command line arguments
    args: Args,

    /// TcpStream for reading and writing to RCON server
    conn: TcpStream,

    /// Last message ID sent to server
    last_sent_id: i32,

    /// Last message ID received from server
    last_received_id: i32,
}

/// RCON possible error states
#[derive(Debug)]
pub enum RconError {
    PacketError,
    AuthError,
    ConnError,
}

pub type RconResult = Result<Rcon, RconError>;

impl Rcon {
    pub fn new(args: Args) -> RconResult {
        let conn = Rcon::get_conn(&args.ip, &args.port);
        let rcon = Rcon {
            args: args,
            conn: conn,
            last_sent_id: 0,
            last_received_id: 0,
        };

        Ok(rcon)
    }

    pub fn get_conn(ip: &str, port: &str) -> TcpStream {
        let conn = TcpStream::connect(format!("{}:{}", ip, port)).expect("Couldn't connect to server at {ip}:{port}");
        conn.set_nonblocking(false).expect("set_nonblocking call failed");
        conn.set_read_timeout(Some(Duration::new(1, 0))).expect("set_read_timeout call failed");
        conn.set_write_timeout(Some(Duration::new(1, 0))).expect("set_write_timeout call failed");
        conn
    }

    // Authenticate RCON session with password
    pub fn authenticate(&mut self) -> bool {
        let pass = rpassword::read_password_from_tty(Some("Password: ")).unwrap();
        let login = Packet::new(0, PacketType::Login, String::from(&pass)).unwrap_or_else(|error| {
            panic!("Could not create login Packet with password: '{:?}'", &pass);
        });

        println!("Authenticating...");
        let resp = self.send_packet(login).unwrap();
        match resp.id {
            0..=i32::MAX if resp.id == self.last_sent_id => true,
            -1 => false,
            _ => {
                eprintln!("Invalid message ID ({}) received from server!", resp.id);
                eprintln!("Unable to authenticate!");
                false
            }
        }
    }

    fn send_packet(&mut self, packet: Packet) -> PacketResult {
        let mut packet_bytes = packet.serialize();
        
        // Send packet
        //println!("<<< Sending packet: {}", packet);
        //println!("Bytes: {:#x}", packet_bytes);
        self.conn.write(packet_bytes.as_mut()).expect("Cannot write data to stream.");
        let mut buf = [0; MAX_PACKET_SIZE];
        
        // Get response from server
        self.conn.read(&mut buf).unwrap();
        let byte_buf = Bytes::copy_from_slice(&buf);
        //println!(">>> Received packet:");
        //println!("Bytes: {:?}", byte_buf);
        //println!("First bytes: {:?}", byte_buf.get(0..20));
        let response = Packet::deserialize(byte_buf);
        return response;
    }

    /// API function to send RCON commands and receive packets
    pub fn send_cmd(&mut self, body: &str) -> PacketResult {
        let packet = Packet::new(0, PacketType::Command, body.to_string()).unwrap();
        self.send_packet(packet)
    }

    pub fn run(mut self) -> io::Result<()> {
        // Authenticate with RCON password
        loop {
            let is_authenticated = self.authenticate();
            if is_authenticated { break };
            println!("Incorrect password. Please try again...");
        }

        
        // Send test "help" command
        let help = self.send_cmd("help").unwrap();
        println!("{}", help);
        println!("{}", "====".repeat(22));

        // Interactive prompt
        let stdin = io::stdin();
        loop {
            let mut line = String::new();
            print!("λ ");
            io::stdout().flush()?;
            stdin.read_line(&mut line)?;
            if line.len() > MAX_PACKET_SIZE - 9 {
                println!("Woah there! That command is waaay too long.");
                println!("You might want to try that again.");
                continue
            }

            let response = self.send_cmd(&line.trim_end());
            println!("{}", response.unwrap());
            println!("{}", "====".repeat(22));
        }
    }
}