/*
 * An interactive RCON shell.
 */

extern crate clap;
extern crate rpassword;
extern crate bytes;

use bytes::{Bytes, BytesMut, Buf, BufMut};
use clap::Parser;
use std::{
    io::{self, Write, Read},
    fmt,
    net::{TcpStream},
    str,
    time::{Duration},
    thread::{sleep},
};

#[derive(Clone)]
enum PacketType {
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
            PacketType::Command => write!(f, "Command"),
            PacketType::Response => write!(f, "Response"),
            _ => write!(f, "UNKNOWN"),
        }
    }
}

const MAX_PACKET_SIZE: usize = 4096;
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

pub type PacketResult = Result<Packet, PacketError>;

impl Packet {
    /// Initialize a packet instance with calculated length and included pad byte
    fn new(id: i32, typ: PacketType, body_text: String) -> PacketResult {
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
            //body_text: replace_color_chars(body_bytes.escape_ascii().to_string()),
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
        write!(f, "Size: {}, ID: {}, Type: {}\nBody: {}", self.size, self.id, self.typ, self.body_text)
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

    let pass = rpassword::read_password_from_tty(Some("Password: ")).unwrap();

    let mut stream = TcpStream::connect(format!("{}:{}", args.host, args.port))
                               .expect("Couldn't connect to server at {args.host}:{args.port}");
    stream.set_read_timeout(Some(Duration::new(1, 0))).expect("set_read_timeout call failed");
    stream.set_write_timeout(Some(Duration::new(1, 0))).expect("sed_write_timeout call failed");
    stream.set_nonblocking(false).expect("set_nonblocking call failed");

    println!("Connected to [{}:{}]", args.host, args.port);
    println!("Authenticating...");

    
    // Setup auth sequence
    let mut send_queue = Vec::<&Packet>::new();
    let login = Packet::new(0, PacketType::Login, pass).unwrap();
    let cmd = Packet::new(1, PacketType::Command, String::from("help")).unwrap();

    send_queue.push(&login);
    send_queue.push(&cmd);
    
    // Send loop
    while send_queue.len() > 0 {
        // Sending packets
        let p = send_queue.remove(0);
        send_packet(p, &stream)?;

        //sleep(Duration::from_millis(1000));

        // Monitor stream sent data size
        //let mut size_buf = [0; 4];
        //let mut packet_size = 0;
        //let count = 0;
        //loop {
        //    stream.read_exact(&mut size_buf)?;
        //    packet_size = Bytes::copy_from_slice(&size_buf).get_i32_le();
        //    if packet_size > 0 { break; }

        //    if count % 50 == 0 { println!("{}: next read...", count) };
        //    count += 50;
        //}
        
        // Receive reponses
        let mut buf = [0; MAX_PACKET_SIZE];
        stream.read(&mut buf)?;
        //stream.read(byte_buf)?;
        let byte_buf = Bytes::copy_from_slice(&buf);
        println!(">>> Received packet:");
        println!("First bytes: {:?}", byte_buf.get(0..20));
        let response = Packet::deserialize(byte_buf).unwrap();
        println!("{}", response);
    }

    //sleep(Duration::from_secs(3));
    Ok(())
}
