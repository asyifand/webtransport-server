use anyhow::Result;
use std::fs;
use std::result::Result::Ok;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::task;
use tracing::error;
use tracing::info;
use tracing::info_span;
use tracing::Instrument;
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::EnvFilter;
use wtransport::endpoint::IncomingSession;
use wtransport::Connection;
use wtransport::Endpoint;
use wtransport::Identity;
use wtransport::ServerConfig;

#[tokio::main]
async fn main() -> Result<()> {
    init_logging();

    // let identity = Identity::load_pemfiles("server.crt", "server.key").await?;
    let identity = Identity::load_pemfiles("certificate.pem", "certificate.key").await.expect("Failed to load cert");
    // let identity = Identity::load_pemfiles("/etc/ssl/certs/mkcert_development_CA_263446088470421260956769455254426110751.pem");

    // let config = ServerConfig::builder()
    //     .with_bind_address("0.0.0.0:4433".parse().unwrap())
    //     .with_identity(identity)
    //     .keep_alive_interval(Some(Duration::from_secs(3)))
    //     .build();

    let config = ServerConfig::builder()
        .with_bind_address("127.0.0.1:4433".parse().unwrap()) // Pastikan client pakai localhost/IP server
        .with_identity(identity)
        .keep_alive_interval(Some(Duration::from_secs(3))) // Pastikan koneksi tetap aktif
        .build();
    // let config = ServerConfig::builder()
    //     .with_bind_default(4433)
    //     .with_identity(Identity::self_signed(["localhost"]).unwrap())
    //     .keep_alive_interval(Some(Duration::from_secs(3)))
    //     .build();

    let server = Endpoint::server(config)?;

    info!("Server ready!");

    let address = server.local_addr()?;
    let ipv4_address = match address {
        std::net::SocketAddr::V4(addr) => addr.to_string(),
        std::net::SocketAddr::V6(addr) => format!("{} (IPv6)", addr),
    };
    info!("Server running on {}", ipv4_address);

    // let server = Endpoint::server(config)?;
    // let server = Arc::new(server); // Bungkus dalam Arc
    // let connections = Arc::new(Mutex::new(Vec::<Connection>::new())); // Simpan koneksi aktif

    info!("Server ready! Listening on {}", server.local_addr()?);

    // task for send automatic message
    // let connections_clone = Arc::clone(&connections);
    // tokio::spawn(async move {
    //     loop {
    //         tokio::time::sleep(Duration::from_secs(1)).await;
    //         let message = format!("Message from server at {:?}", std::time::SystemTime::now());
    //         info!("Auto-Sent: {}", message);

    //         let mut conns = connections_clone.lock().await;
    //         for conn in conns.iter() {
    //             if let Ok(stream_fut) = conn.open_uni().await {
    //                 if let Ok(mut stream) = stream_fut.await {
    //                     let _ = stream.write_all(message.as_bytes()).await;
    //                 }
    //             }
    //         }
    //     }
    // });

    // Looping for get new connection
    // let connections_clone = Arc::clone(&connections);
    // for id in 0.. {
    //     let incoming_session = server.accept().await;
    //     let connections_clone = Arc::clone(&connections_clone);
    //     tokio::spawn(async move {
    //         if let Ok(conn) = handle_connection(incoming_session).await {
    //             connections_clone.lock().await.push(conn);
    //         }
    //     });
    // }

    // Ok(())
    for id in 0.. {
        let incoming_session = server.accept().await;
        tokio::spawn(handle_connection(incoming_session).instrument(info_span!("Connection", id)));
    }

    Ok(())
}

async fn handle_connection(incoming_session: IncomingSession) {
    let result = handle_connection_impl(incoming_session).await;
    error!("{:?}", result);
}

async fn handle_connection_impl(incoming_session: IncomingSession) -> Result<()> {
    let mut buffer = vec![0; 65536].into_boxed_slice();

    info!("Waiting for session request...");

    let session_request = incoming_session.await?;

    info!(
        "New session: Authority: '{}', Path: '{}'",
        session_request.authority(),
        session_request.path()
    );

    let connection = session_request.accept().await?;

    info!("Waiting for data from client...");

    // Send data 100 times
    let connection_clone = connection.clone();
    task::spawn(async move {
        for i in 1..=1000 {
            tokio::time::sleep(Duration::from_millis(500)).await;

            match connection_clone.open_uni().await {
                Ok(stream_fut) => match stream_fut.await {
                    Ok(mut stream) => {
                        let message = format!("Message {} from server", i);
                        if let Err(e) = stream.write_all(message.as_bytes()).await {
                            error!("Failed to send message {}: {:?}", i, e);
                        } else {
                            info!("Sent: {}", message);
                        }
                    }
                    Err(e) => error!("Failed to open unidirectional stream: {:?}", e),
                },
                Err(e) => error!("Failed to create stream future: {:?}", e),
            }
        }
        info!("Finished sending 100 messages.");
    });

    loop {
        tokio::select! {
            stream = connection.accept_bi() => {
                let mut stream = stream?;
                info!("Accepted BI stream");

                let bytes_read = match stream.1.read(&mut buffer).await? {
                    Some(bytes_read) => bytes_read,
                    None => continue,
                };

                let str_data = std::str::from_utf8(&buffer[..bytes_read])?;

                info!("Received (bi) '{str_data}' from client");

                stream.0.write_all(b"ACK").await?;
            }
            stream = connection.accept_uni() => {
                let mut stream = stream?;
                info!("Accepted UNI stream");

                let bytes_read = match stream.read(&mut buffer).await? {
                    Some(bytes_read) => bytes_read,
                    None => continue,
                };

                let str_data = std::str::from_utf8(&buffer[..bytes_read])?;

                info!("Received (uni) '{str_data}' from client");

                let mut stream = connection.open_uni().await?.await?;
                stream.write_all(b"ACK").await?;
            }
            dgram = connection.receive_datagram() => {
                let dgram = dgram?;
                let str_data = std::str::from_utf8(&dgram)?;

                info!("Received (dgram) '{str_data}' from client");

                connection.send_datagram(b"ACK")?;
            }
        }
    }
}

fn init_logging() {
    let env_filter = EnvFilter::builder()
        .with_default_directive(LevelFilter::INFO.into())
        .from_env_lossy();

    tracing_subscriber::fmt()
        .with_target(true)
        .with_level(true)
        .with_env_filter(env_filter)
        .init();
}
