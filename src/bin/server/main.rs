use std::{io, net::SocketAddr};

use tokio::{
    net::{TcpListener, TcpStream},
    spawn,
};

const MAX_MSG_BYTES: usize = 1024;

#[tokio::main]
async fn main() {
    let listener = TcpListener::bind("127.0.0.1:60000")
        .await
        .expect("No client");

    loop {
        let (socket, sock_addr) = listener.accept().await.expect("Cannot accept connection");
        spawn(async move {
            if let Err(e) = handle_bytes(socket, sock_addr).await {
                eprintln!("{}", e);
            };
        });
    }
}

async fn handle_bytes(stream: TcpStream, sock_addr: SocketAddr) -> std::io::Result<()> {
    println!(
        "Incoming connection. Port: {}, address: {}",
        sock_addr.port(),
        sock_addr.ip()
    );

    let mut buf = [0; MAX_MSG_BYTES];
    loop {
        stream.readable().await?;
        match stream.try_read(&mut buf) {
            Ok(0) => {
                println!("Bye {}", sock_addr.port());
                break;
            }
            Ok(n) => {
                println!("Got {} bytes from {}", n, sock_addr.port());
                println!("{}", String::from_utf8_lossy(&buf[..n]));
            }
            // False wake
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                continue;
            }
            Err(e) => {
                return Err(e.into());
            }
        };
    }
    Ok(())
}
