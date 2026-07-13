use axum::{
    body::Body,
    extract::Path,
    http::{
        header::{CONTENT_DISPOSITION, CONTENT_TYPE}, HeaderValue,
        StatusCode,
    },
    response::Response,
    routing::get,
    Router,
};
use bore_cli::client::Client;
use clap::Parser;
use futures_util::StreamExt;
use rand::distr::SampleString;
use std::io::{Read, Write};
use std::process;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;
use tokio_util::io::ReaderStream;

#[derive(Parser, Debug)]
#[command(name = "lift", version, about, long_about = None)]
struct Args {
    /// Transfer the target as a downloadable file instead of rendering as HTML/text
    #[arg(short = 'f')]
    file_mode: bool,

    /// Do not use a remote bore server
    #[arg(short = 'l')]
    local_mode: bool,

    /// The path to the file to share. If omitted, reads text from stdin
    #[arg(value_name = "FILENAME")]
    filename: Option<String>,

    /// How often the file can be accessed before the link expires
    #[arg(
        value_name = "MAX_COUNT",
        short = 'c',
        long = "count",
        default_value = "1"
    )]
    max_count: u32,

    /// The time in seconds after which lift terminates and the link expires, by default never
    #[arg(
        value_name = "TIMEOUT",
        short = 't',
        long = "time",
        default_value = "0"
    )]
    max_time: u64,

    /// The bore server to use
    #[arg(
        value_name = "REMOTE",
        short = 'r',
        long = "remote",
        default_value = "bore.pub"
    )]
    bore_remote: String,

    /// Encrypt with a certain passphrase, or a random one if not specified
    #[arg(value_name = "PASSPHRASE", short = 'p', long = "passphrase", num_args = 0..=1, default_missing_value = ""
    )]
    passphrase: Option<String>,
}

enum Payload {
    Text(String),
    File { bytes: Vec<u8>, filename: String },
}

fn random_path() -> String {
    rand::distr::Alphanumeric.sample_string(&mut rand::rng(), 8)
}

fn local_ip() -> String {
    use std::net::UdpSocket;
    UdpSocket::bind("0.0.0.0:0")
        .and_then(|socket| {
            socket.connect("8.8.8.8:80")?;
            socket.local_addr()
        })
        .map(|addr| addr.ip().to_string())
        .unwrap_or_else(|_| "0.0.0.0".to_string())
}

async fn serve(
    payload: Payload,
    max_count: u32,
    max_seconds: u64,
    bore_remote: String,
    local: bool,
) {
    let r = random_path();

    let listener = tokio::net::TcpListener::bind("0.0.0.0:0").await.unwrap();
    let local_port = listener.local_addr().unwrap().port();

    let display_addr = if local {
        format!("{}:{}", local_ip(), local_port)
    } else {
        let client = Client::new("localhost", local_port, &bore_remote, 0, None)
            .await
            .expect("Failed to create bore");
        let remote_port = client.remote_port();
        tokio::spawn(async move { client.listen().await });
        format!("{}:{}", bore_remote, remote_port)
    };

    let shutdown = Arc::new(Notify::new());
    let counter = Arc::new(AtomicU32::new(max_count));
    let payload = Arc::new(payload);

    let app = Router::new().route(
        "/{seg}",
        get({
            let path = r.clone();
            let shutdown = shutdown.clone();

            move |Path(seg): Path<String>| {
                let path = path.clone();
                let payload = payload.clone();
                let counter = counter.clone();
                let shutdown = shutdown.clone();

                async move {
                    if seg != path {
                        return Err(StatusCode::NOT_FOUND);
                    }

                    let prev = counter.try_update(Ordering::SeqCst, Ordering::SeqCst, |v| {
                        if v > 0 { Some(v - 1) } else { None }
                    });

                    let prev = match prev {
                        Err(_) => return Err(StatusCode::NOT_FOUND),
                        Ok(val) => val,
                    };

                    Ok(match &*payload {
                        Payload::File { bytes, filename } => {
                            let bytes = bytes.clone();
                            let stream = async_stream::stream! {
                                let cursor = std::io::Cursor::new(bytes);
                                let mut reader = ReaderStream::new(cursor);
                                while let Some(chunk) = reader.next().await {
                                    yield chunk;
                                }
                                if prev == 1 {
                                    shutdown.notify_one();
                                }
                            };
                            Response::builder()
                                .header(CONTENT_TYPE, "application/octet-stream")
                                .header(
                                    CONTENT_DISPOSITION,
                                    HeaderValue::from_str(&format!(
                                        "attachment; filename=\"{}\"",
                                        filename
                                    ))
                                    .unwrap(),
                                )
                                .body(Body::from_stream(stream))
                                .unwrap()
                        }
                        Payload::Text(text) => {
                            if prev == 1 {
                                shutdown.notify_one();
                            }
                            Response::builder()
                                .header(CONTENT_TYPE, "text/html; charset=utf-8")
                                .body(Body::from(text.clone()))
                                .unwrap()
                        }
                    })
                }
            }
        }),
    );

    println!("Data available at http://{}/{}", display_addr, r);

    if max_seconds > 0 {
        println!(
            "Expires after {} download(s) or {}s.",
            max_count, max_seconds
        );
    } else {
        println!("Expires after {} download(s).", max_count);
    }

    let run = axum::serve(listener, app).with_graceful_shutdown(async move {
        shutdown.notified().await;
        println!(
            "Maximum request count of {} reached, not accepting further connections.",
            max_count
        );
    });

    if max_seconds > 0 {
        if tokio::time::timeout(Duration::from_secs(max_seconds), run)
            .await
            .is_err()
        {
            println!(
                "Timeout of {}s reached, not accepting further connections.",
                max_seconds
            );
            return;
        }
    } else {
        run.await.unwrap();
    }

    // extra time for tcp buffer to clear, as we only know that the buffer was transferred to OS
    tokio::time::sleep(Duration::from_secs(3)).await;
    println!("Transfer complete, closing remote.");
}

fn encrypt_bytes_with_passphrase(data: &[u8], passphrase: &str) -> Vec<u8> {
    let secret = age::secrecy::SecretString::from(passphrase.to_string());
    let encryptor = age::Encryptor::with_user_passphrase(secret);

    let mut encrypted = Vec::new();
    let mut writer = encryptor
        .wrap_output(&mut encrypted)
        .expect("Failed to start encryption");
    writer.write_all(data).expect("Failed to encrypt data");
    writer.finish().expect("Failed to finalize encryption");

    encrypted
}

fn from_stdin() -> String {
    let mut s = String::new();
    std::io::stdin()
        .read_to_string(&mut s)
        .expect("Failed to read stdin");
    s
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    let passphrase: Option<String> = match args.passphrase {
        Some(p) if !p.is_empty() => Some(p),
        Some(_) => {
            let generated = rand::distr::Alphanumeric.sample_string(&mut rand::rng(), 16);
            println!("Generated passphrase: {}", generated);
            println!(
                "Share this with the receiver. \
                They can decrypt the file using 'age -d -p -o the_file.sth the_file.sth.age'."
            );
            Some(generated)
        }
        None => None,
    };

    let payload = match args.filename {
        None => {
            if args.file_mode {
                eprintln!("Error: -f requires a filename");
                process::exit(1);
            }
            Payload::Text(from_stdin())
        }
        Some(path) => {
            if args.file_mode {
                Payload::File {
                    bytes: match std::fs::read(&path) {
                        Ok(b) => b,
                        Err(e) => {
                            eprintln!("Error: {}", e);
                            process::exit(1);
                        }
                    },
                    filename: std::path::Path::new(&path)
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                }
            } else {
                Payload::Text(match std::fs::read_to_string(&path) {
                    Ok(s) => s,
                    Err(e) => {
                        eprintln!("Error: {}", e);
                        process::exit(1);
                    }
                })
            }
        }
    };

    let payload = if let Some(pass) = &passphrase {
        match payload {
            Payload::Text(text) => {
                let encrypted = encrypt_bytes_with_passphrase(text.as_bytes(), pass);
                Payload::File {
                    bytes: encrypted,
                    filename: "message.txt.age".to_string(),
                }
            }
            Payload::File { bytes, filename } => {
                let encrypted = encrypt_bytes_with_passphrase(&bytes, pass);
                Payload::File {
                    bytes: encrypted,
                    filename: format!("{}.age", filename),
                }
            }
        }
    } else {
        payload
    };

    serve(
        payload,
        args.max_count,
        args.max_time,
        args.bore_remote,
        args.local_mode,
    )
    .await;
}
