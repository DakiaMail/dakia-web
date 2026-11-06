use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    time::Duration,
};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr: SocketAddr = "127.0.0.1:3025".parse()?;
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(3))?;
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    stream.set_write_timeout(Some(Duration::from_secs(3)))?;
    stream.write_all(b"GET /healthz HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")?;
    let mut response = String::new();
    stream.take(8192).read_to_string(&mut response)?;
    if !response.starts_with("HTTP/1.1 200 ") || !response.contains("\"service\":\"dakia-web\"") {
        return Err("Dakia health identity check failed".into());
    }
    Ok(())
}
