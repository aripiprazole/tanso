#![feature(async_fn_in_trait)]
#![feature(box_syntax)]

use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

trait PacketEncoder {
    fn packet_id() -> i32;

    async fn encode<W: AsyncWrite + Unpin>(&self, stream: &mut W) -> std::io::Result<()>;
}

trait PacketDecoder {
    async fn decode<R: AsyncRead + Unpin>(id: i32, stream: &mut R) -> std::io::Result<Box<Self>>;
}

#[derive(Debug)]
struct Handshake {
    protocol_version: i32,
    server_address: String,
    server_port: u16,
    next_state: i32,
}

impl PacketDecoder for Handshake {
    async fn decode<R: AsyncRead + Unpin>(_id: i32, stream: &mut R) -> std::io::Result<Box<Self>> {
        let protocol_version = read_var_int(stream).await?;
        let server_address = read_string(stream).await?;
        let server_port = stream.read_u16().await?;
        let next_state = read_var_int(stream).await?;

        Ok(box Handshake {
            protocol_version,
            server_address,
            server_port,
            next_state,
        })
    }
}

#[derive(Debug)]
struct Ping {
    payload: u64,
}

impl PacketEncoder for Ping {
    fn packet_id() -> i32 { 0 }

    async fn encode<W: AsyncWrite + Unpin>(&self, stream: &mut W) -> std::io::Result<()> {
        stream.write_u64(self.payload).await?;

        Ok(())
    }
}

impl PacketDecoder for Ping {
    async fn decode<R: AsyncRead + Unpin>(_id: i32, _stream: &mut R) -> std::io::Result<Box<Self>> {
        let start = SystemTime::now();
        let now = start
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards");

        let payload = now.as_secs();

        Ok(box Ping { payload })
    }
}

#[derive(Debug)]
struct Response {
    value: String,
}

impl PacketEncoder for Response {
    fn packet_id() -> i32 { 0 }

    async fn encode<W: AsyncWrite + Unpin>(&self, stream: &mut W) -> std::io::Result<()> {
        write_string(stream, self.value.clone()).await?;

        Ok(())
    }
}

async fn write_var_int<W: AsyncWrite + Unpin>(stream: &mut W, mut value: i32) -> std::io::Result<()> {
    loop {
        if value & 0xfffff80 == 0 {
            stream.write_u8(value as u8).await?;
            break;
        }

        stream.write_u8((value & 0x7f | 0x80) as u8).await?;
        value >>= 7;
    }

    Ok(())
}

async fn read_var_int<R: AsyncRead + Unpin>(stream: &mut R) -> std::io::Result<i32> {
    let mut offset = 0;
    let mut value = 0;

    loop {
        let byte = stream.read_u8().await?;
        value |= ((byte as i32 & 0x7f) << offset) as i32;

        offset += 7;

        if byte & 0x80 == 0 { break; }
        if offset >= 32 { panic!("too long var int") }
    }

    Ok(value)
}

async fn write_string<W: AsyncWrite + Unpin>(stream: &mut W, value: String) -> std::io::Result<()> {
    let bytes = value.as_bytes();

    write_var_int(stream, bytes.len() as i32).await?;
    stream.write_all(bytes).await?;

    Ok(())
}

async fn read_string<R: AsyncRead + Unpin>(stream: &mut R) -> std::io::Result<String> {
    let length = read_var_int(stream).await?;
    let mut buf = vec![0; length as usize];

    stream.read_exact(&mut buf).await?;

    Ok(String::from_utf8(buf).unwrap())
}

async fn read_packet<P: PacketDecoder>(stream: &mut TcpStream) -> std::io::Result<P> {
    let _packet_length = read_var_int(stream).await?;
    let packet_id = read_var_int(stream).await?;

    let packet = P::decode(packet_id, stream).await?;

    Ok(*packet)
}

async fn write_packet<P: PacketEncoder>(stream: &mut TcpStream, packet: &P) -> std::io::Result<()> {
    let mut buf: Vec<u8> = vec![];

    write_var_int(&mut buf, P::packet_id()).await?;
    packet.encode(&mut buf).await?;

    write_var_int(stream, buf.len() as i32).await?;

    stream.write(&mut buf).await?;

    Ok(())
}

const RESPONSE: &str = r#"
{
    "version": {
        "name": "1.18.2",
        "protocol": 758
    },
    "players": {
        "max": 100,
        "online": 0,
        "sample": []
    },
    "description": {
        "text": "A Minecraft Server"
    }
}
"#;

async fn process_status(mut stream: TcpStream) -> std::io::Result<()> {
    read_packet::<Handshake>(&mut stream).await?;
    write_packet(&mut stream, &Response { value: RESPONSE.to_string() }).await?;

    let ping = read_packet::<Ping>(&mut stream).await?;
    write_packet(&mut stream, &ping).await?;

    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let listener = TcpListener::bind("127.0.0.1:8080").await?;

    loop {
        let (socket, _) = listener.accept().await?;

        tokio::spawn(async move {
            // Process each socket concurrently.
            if let Err(err) = process_status(socket).await {
                eprintln!("error processing socket; err = {:?}", err);
            }
        });
    }
}
