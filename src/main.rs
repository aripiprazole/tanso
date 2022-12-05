#![feature(async_fn_in_trait)]
#![feature(box_syntax)]

use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::net::{TcpListener, TcpStream};

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

async fn read_string<R: AsyncRead + Unpin>(stream: &mut R) -> std::io::Result<String> {
    let length = read_var_int(stream).await?;
    let mut buf = vec![0; length as usize];

    stream.read_exact(&mut buf).await?;

    Ok(String::from_utf8(buf).unwrap())
}

async fn read_packet<P: PacketDecoder>(stream: &mut TcpStream) -> std::io::Result<P> {
    let mut stream = stream;

    let _packet_length = read_var_int(stream).await?;
    let packet_id = read_var_int(stream).await?;

    let packet = P::decode(packet_id, &mut stream).await?;

    Ok(*packet)
}

async fn process_status(mut stream: TcpStream) -> std::io::Result<()> {
    let handshake = read_packet::<Handshake>(&mut stream).await?;

    println!("{:?}", handshake);

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
