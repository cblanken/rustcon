/*
 * An interactive RCON shell.
 */

use bytes::{Buf, BufMut, Bytes, BytesMut};
use clap::Parser;
use std::{
    env, fmt,
    io::{self, stdin, stdout, Read, Write},
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

/// Definition for
///
/// Source: [https://developer.valvesoftware.com/wiki/Source_RCON_Protocol#Packet_Type](https://developer.valvesoftware.com/wiki/Source_RCON_Protocol#Packet_Type)
#[derive(Clone)]
pub enum PacketType {
    /// `SERVERDATA_AUTH`
    Login = 3,
    /// `SERVERDATA_EXECCOMMAND` or `SERVERDATA_AUTH_RESPONSE`
    Command = 2,
    /// `SERVERDATA_RESPONSE_VALUE`
    Response = 0,
    /// A packet type that doesn't follow the RCON specification
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

const PACKET_SIZE_FIELD_LEN: usize = 4;
const PACKET_SIZE_MIN: usize = 10;
const PACKET_SIZE_MAX: usize = 4096;
const PACKET_BODY_MAX_LEN: usize = PACKET_SIZE_MAX - PACKET_SIZE_MIN;
const PACKET_MAX_BUFFER_LEN: usize = PACKET_SIZE_FIELD_LEN + PACKET_SIZE_MAX;
const BAD_AUTH: i32 = -1;

/// RCON packet structure
///
/// Source: [https://developer.valvesoftware.com/wiki/Source_RCON_Protocol#Basic_Packet_Structure](https://developer.valvesoftware.com/wiki/Source_RCON_Protocol#Basic_Packet_Structure)
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
    /// Initialize a packet instance with calculated length and a pad byte
    pub fn new(id: i32, typ: PacketType, body_text: String) -> PacketResult {
        let body_bytes = Bytes::from(body_text.trim_end().to_string().clone());
        if !body_bytes.is_ascii() {
            Err(PacketError::NonAscii)
        } else {
            let packet = Packet {
                size: body_bytes.len() as i32 + 10,
                id,
                typ,
                body_text,
                body_bytes,
                pad: 0,
            };

            Ok(packet)
        }
    }

    fn replace_color_codes(s: String) -> String {
        let mut filtered = String::new();
        let mut iter = s.chars();
        while let Some(ch) = iter.next() {
            if ch == '§' {
                iter.next();
            } else {
                filtered.push(ch);
            }
        }
        filtered
    }

    fn deserialize(bytes: &mut Bytes) -> PacketResult {
        let size = bytes.get_i32_le();
        let id = bytes.get_i32_le();
        let typ = PacketType::from(bytes.get_i32_le());

        // Copy out bytes from body up to max possible packet size
        let body_size = match size as usize {
            0..=9 => Err(PacketError::SmallPacket)?,
            PACKET_SIZE_MIN..=PACKET_SIZE_MAX => size as usize - PACKET_SIZE_MIN,
            _ => PACKET_BODY_MAX_LEN,
        };

        let body_bytes = bytes.copy_to_bytes(body_size);

        let packet = Packet {
            size,
            id,
            typ,
            body_text: {
                Packet::replace_color_codes(
                    str::from_utf8(&body_bytes)
                        .unwrap_or_else(|_body| {
                            eprintln!("Could not parse the body as UTF-8");
                            eprintln!("Here are the raw bytes:\n{:#?}", body_bytes);
                            ""
                        })
                        .to_string(),
                )
            },
            body_bytes,
            pad: 0,
        };
        Ok(packet)
    }

    /// Serialize packet into a Vec<u8>
    fn serialize(&self) -> BytesMut {
        let mut p = BytesMut::with_capacity(PACKET_SIZE_MAX);

        // Construct packet data in bytes
        p.put_i32_le(self.size);
        p.put_i32_le(self.id);
        p.put_i32_le(self.typ.clone() as i32);
        p.put(self.body_bytes.clone());
        p.put_u8('\0' as u8); // terminate body with null byte
        p.put_u8(self.pad); // append pad null byte
        return p;
    }
}

impl fmt::Display for Packet {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "Size: {} bytes, ID: {}, Type: {}\n{}",
            self.size, self.id, self.typ, self.body_text
        )
    }
}

/// RCON connection struct for handling sending and receiving RCON packets
pub struct Rcon {
    /// TcpStream for reading and writing to RCON server
    conn: TcpStream,

    /// Last message ID sent to server
    last_sent_id: i32,

    /// Next message ID to send
    next_send_id: i32,
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
            next_send_id: 1,
        };

        Ok(rcon)
    }

    pub fn get_conn(ip: &str, port: &str) -> io::Result<TcpStream> {
        let conn = TcpStream::connect(format!("{}:{}", ip, port));
        match conn {
            Ok(c) => {
                c.set_nonblocking(false)
                    .expect("set_nonblocking call failed");
                c.set_read_timeout(Some(Duration::new(1, 0)))
                    .expect("set_read_timeout call failed");
                c.set_write_timeout(Some(Duration::new(1, 0)))
                    .expect("set_write_timeout call failed");
                Ok(c)
            }
            Err(e) => Err(e),
        }
    }

    fn authenticate_with(&mut self, pass: String) -> bool {
        let login = Packet::new(1, PacketType::Login, String::from(&pass));
        if let Ok(packet) = login {
            if let Err(e) = self.send_packet(packet) {
                eprintln!("Failed to send login Packet. Error: {:?}", e);
                return false;
            }
            if let Ok(auth_response) = self.receive_packets() {
                // Check all received packets for invalid auth since SRCDS sends multiple packets for auth response
                for p in &auth_response {
                    if p.id == BAD_AUTH || p.id != self.last_sent_id {
                        return false;
                    }
                }

                // Send followup packet, SRCDS doesn't accept the first command after auth
                self.send_cmd("").unwrap();
                self.receive_packets().unwrap();
                return true;
            } else {
                return false;
            }
        } else {
            eprintln!("The password: \"{pass}\" is invalid. RCON only supports ASCII text.");
            return false;
        }
    }

    // Authenticate RCON session with password
    pub fn authenticate(&mut self) -> bool {
        let pass = rpassword::read_password_from_tty(Some("Password: ")).unwrap_or_else(|_| {
            eprintln!("RCON passwords can only be ASCII text.");
            eprintln!("Please try again.");
            "".to_string()
        });
        self.authenticate_with(pass)
    }

    fn send_packet(&mut self, packet: Packet) -> Result<i32, RconError> {
        let mut packet_bytes = packet.serialize();

        // Send packet
        if let Err(e) = self.conn.write(packet_bytes.as_mut()) {
            eprintln!("{}", e);
            return Err(RconError::ConnError);
        }

        self.last_sent_id = packet.id;
        self.next_send_id = self.last_sent_id + 1;
        Ok(self.last_sent_id)
    }

    fn receive_packets(&mut self) -> Result<Vec<Packet>, RconError> {
        let mut packets: Vec<Packet> = Vec::new();
        let mut vec_buf: Vec<u8> = vec![0; PACKET_MAX_BUFFER_LEN];

        // TODO try refactoring with TcpStream.read_to_end()
        // An error shows up when running long commands that return 3+ packets
        // which give me weird reads (not filling out buffer or reading too far)
        // Pretty sure it's because the TcpStream.read() is completing reads NOT
        // on packet divisions and the "next packet" get's a bad length value when
        // it gets deserialized

        // Read all available packets
        while let Ok(_) = self.conn.read(&mut vec_buf) {
            // Retrieve all packets
            let mut byte_buf = Bytes::copy_from_slice(&vec_buf);
            let response = Packet::deserialize(&mut byte_buf);

            match response {
                Ok(r) => {
                    // Handle auth double packet response from SRCDS
                    if r.id == BAD_AUTH {
                        packets.push(r);
                        return Ok(packets);
                    } else {
                        packets.push(r);
                    }
                }
                Err(PacketError::SmallPacket) => return Err(RconError::PacketError),
                Err(PacketError::NonAscii) => return Err(RconError::PacketError),
            }
        }

        Ok(packets)
    }

    /// Send an RCON command and receive response packets
    pub fn send_cmd(&mut self, body: &str) -> Result<Vec<Packet>, RconError> {
        let packet = Packet::new(self.next_send_id, PacketType::Command, body.to_string()).unwrap();
        self.send_packet(packet)?;
        self.receive_packets()

        // TODO (might be SRCDS specific)
        // Send follow-up SERVERDATA_RESPONSE_VALUE packet
        // This causes the server the server to respond with an empty packet body
        // when all the response packets have been received for a given command
    }

    /// Launch interactive shell to send RCON commands and receive responses
    pub fn shell(mut self) -> RconResult {
        println!("Authenticating...");
        // Try RUSTCON_PASS env variable
        let env_var_is_valid = match env::var("RUSTCON_PASS") {
            Ok(pass) => self.authenticate_with(pass),
            Err(_) => {
                println!("RUSTCON_PASS env variable does not exist");
                false
            }
        };

        // Try password from user
        if !env_var_is_valid {
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
                return Err(RconError::ConnError);
            }
            if let Err(e) = stdin.read_line(&mut line) {
                eprintln!("{}", e);
                return Err(RconError::ConnError);
            }

            if line.len() > PACKET_SIZE_MAX - 9 {
                eprintln!("Woah there! That command is waaay too long.");
                eprintln!("You might want to try that again.");
                continue;
            }

            let cmd = &line.trim_end();
            if cmd == &"exit" || cmd == &"quit" {
                println!("Sending {:?} could cause the server to shut down.", cmd);
                println!("Type Ctrl+C to close the RCON console");
                println!("{}", "=".repeat(80));
                continue;
            }
            if let Ok(response) = self.send_cmd(cmd) {
                for p in response {
                    println!("{}", p);
                }
            } else {
                eprintln!("Unable to send the command: {cmd}");
                eprintln!("There may have been a connection error. Please try again.");
                return Err(RconError::ConnError);
            }

            println!("{}", "=".repeat(80));
        }
    }
}
