/*
 * An interactive RCON shell.
 */

use bytes::{Bytes, BytesMut, Buf, BufMut};
use clap::Parser;
use std::{
    env,
    io::{self, stdin, stdout, Read, Write},
    fmt,
    net::TcpStream,
    str,
    time::Duration,
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
const MAX_PACKET_BODY_SIZE: usize = MAX_PACKET_SIZE - 12;
const BAD_AUTH: i32 = -1;

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
    SmallPacket,
    NonAscii,
}

type PacketResult = Result<Packet, PacketError>;

impl Packet {
    /// Initialize a packet instance with calculated length and included pad byte
    pub fn new(id: i32, typ: PacketType, body_text: String) -> PacketResult {
        let body_bytes = Bytes::from(body_text.clone());
        if !body_bytes.is_ascii() {
            Err(PacketError::NonAscii)
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
        let mut filtered_s = String::new();
        let mut iter = s.chars();
        while let Some(ch) = iter.next() {
            if ch == '§' {
                iter.next();
            } else {
                filtered_s.push(ch);
            }
        }
        filtered_s
    }

    fn deserialize(mut bytes: Bytes) -> PacketResult {
        let size = bytes.get_i32_le();
        //println!("packet size: {}", size);
        let id = bytes.get_i32_le();
        let typ = PacketType::from(bytes.get_i32_le());
        
        // Copy out bytes from body up to max possible packet size
        let body_size = match size as usize {
            0..=9 => Err(PacketError::SmallPacket)?,
            10..=MAX_PACKET_SIZE => size as usize - 9,
            _ => MAX_PACKET_BODY_SIZE,
        };

        let body_bytes = bytes.copy_to_bytes(body_size);

        let packet = Packet {
            size: size,
            id: id,
            typ: typ,
            body_text: {
                Packet::replace_color_codes(str::from_utf8(&body_bytes)
                    .unwrap_or_else(|_body| {
                        eprintln!("Could not parse the body bytes as UTF-8");
                        eprintln!("Here are the raw bytes:\n{:#?}", body_bytes);
                        ""
                    })
                    .to_string())
            },
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

/// RCON connection struct for handling sending and receiving RCON packets
pub struct Rcon {
    /// TcpStream for reading and writing to RCON server
    conn: TcpStream,

    /// Last message ID sent to server
    last_sent_id: i32,
}

/// RCON session error
#[derive(Debug)]
pub enum RconError {
    PacketError,
    AuthError,
    ConnError,
}

pub type RconResult = Result<Rcon, RconError>;

impl Rcon {
    pub fn new(args: &Args) -> RconResult {
        let conn = Rcon::get_conn(&args.ip, &args.port);
        let rcon = Rcon {
            conn: match conn {
                Ok(c) => c,
                Err(_) => return Err(RconError::ConnError),
            },
            last_sent_id: 0,
        };

        Ok(rcon)
    }

    pub fn get_conn(ip: &str, port: &str) -> io::Result<TcpStream> {
        let conn = TcpStream::connect(format!("{}:{}", ip, port));
        match conn {
            Ok(c) => {
                c.set_nonblocking(false).expect("set_nonblocking call failed");
                c.set_read_timeout(Some(Duration::new(1, 0))).expect("set_read_timeout call failed");
                c.set_write_timeout(Some(Duration::new(1, 0))).expect("set_write_timeout call failed");
                Ok(c)
            },
            Err(e) => Err(e),
        }
    }

    fn authenticate_with(&mut self, pass: String) -> bool {
        let login = Packet::new(1, PacketType::Login, String::from(&pass));
        if let Ok(packet) = login {
            if let Err(e) = self.send_packet(packet) {
                eprintln!("Failed to send login Packet. Error: {:?}", e);
                return false
            }
            if let Ok(auth_response) = self.receive_packets() {
                // Check all received packets for invalid auth since SRCDS sends multiple packets for auth response
                for p in &auth_response {
                    if p.id == BAD_AUTH || p.id != self.last_sent_id {
                        return false;
                    }
                }
                return true;
            } else {
                return false;
            }
            //println!(">>> Received AUTH response:");
        } else {
            eprintln!("Failed to create login Packet with password: '{:?}'", &pass);
            return false
        }
    }

    // Authenticate RCON session with password
    pub fn authenticate(&mut self) -> bool {
        let pass = rpassword::read_password_from_tty(Some("Password: ")).unwrap();
        self.authenticate_with(pass)
    }

    fn send_packet(&mut self, packet: Packet) -> Result<i32, RconError>{
        let mut packet_bytes = packet.serialize();
        
        // Send packet
        //println!("<<< Sending packet: {}", packet);
        //println!("Bytes: {:#x}", packet_bytes);
        self.conn.write(packet_bytes.as_mut()).expect("Cannot write data to stream.");

        self.last_sent_id = packet.id;
        Ok(self.last_sent_id)
    }

    fn receive_packets(&mut self) -> Result<Vec::<Packet>, RconError> {
        let mut packets: Vec::<Packet> = Vec::new();
        let mut buf = [0; MAX_PACKET_SIZE];
       
        // TODO try refactoring with TcpStream.read_to_end()
        // An error shows up when running long commands that return 3+ packets
        // which give me weird reads (not filling out buffer or reading too far)
        // Pretty sure it's because the TcpStream.read() is completing reads NOT
        // on packet divisions and the "next packet" get's a bad length value when
        // it gets deserialized
       
        //let mut vec_buf: Vec::<u8> = Vec::new();
        //self.conn.read_to_end(&mut vec_buf).unwrap();
        //let new_buf = Bytes::copy_from_slice(&vec_buf);

        // Read all available packets
        while let Ok(_) = self.conn.read(&mut buf) {
            // Retrieve all packets
            let byte_buf = Bytes::copy_from_slice(&buf);
            //println!(">>> Received packet:");
            //println!("Bytes: {:?}", byte_buf);
            //println!("First bytes: {:?}", byte_buf.get(0..20));
            let response = Packet::deserialize(byte_buf);
            
            match response {
                Ok(r) => {
                    // Handle double auth packet from SRCDS
                    if r.body_bytes.len() == 0 || r.id == BAD_AUTH {
                        packets.push(r);
                        return Ok(packets);
                    } else {
                        packets.push(r);
                    }
                },
                Err(PacketError::SmallPacket) => {
                    return Err(RconError::PacketError)
                },
                Err(PacketError::NonAscii) => {
                    return Err(RconError::PacketError)
                }
            }
        }

        Ok(packets)
    }

    /// API function to send RCON commands and receive packets
    pub fn send_cmd(&mut self, body: &str) -> Result<Vec::<Packet>, RconError> {
        let packet = Packet::new(self.last_sent_id + 1, PacketType::Command, body.to_string()).unwrap();
        self.send_packet(packet)?;
        self.receive_packets()
        
        // TODO (might be SRCDS specific)
        // Send follow-up SERVERDATA_RESPONSE_VALUE packet 
        // This causes the server the server to respond with an empty packet body
        // when all the response packets have been received for a given command
    }

    pub fn run(mut self) -> RconResult {
        println!("Authenticating...");
        // Try RUSTCON_PASS env variable but default to empty string
        if self.authenticate_with(env::var("RUSTCON_PASS").unwrap_or("".to_string())) {}
        // Try password from user
        else {
            while !self.authenticate() {
                println!("Incorrect password. Please try again...");
            }
        }

        // Interactive prompt
        println!("{}", "=".repeat(80));
        let stdin = stdin();

        loop {
            let mut line = String::new();
            
            // Set prompt and read user commands
            print!("λ: ");
            if let Err(e) = stdout().flush() {
                eprintln!("{}", e);
                return Err(RconError::ConnError)
            }
            if let Err(e) = stdin.read_line(&mut line) {
                eprintln!("{}", e);
                return Err(RconError::ConnError)
            }

            if line.len() > MAX_PACKET_SIZE - 9 {
                eprintln!("Woah there! That command is waaay too long.");
                eprintln!("You might want to try that again.");
                continue
            }

            if let Ok(response) = self.send_cmd(&line.trim_end()) {
                for p in response {
                    println!("{}", p);
                }
            } else {
                return Err(RconError::ConnError);
            }

            println!("{}", "=".repeat(80));
        }
    }
}
