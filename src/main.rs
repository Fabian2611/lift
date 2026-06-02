use bore_cli::client::Client;
use clap::Parser;
use rand::distr::SampleString;
use std::io::Read;
use std::process;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;
use warp::http::header::{CONTENT_DISPOSITION, CONTENT_TYPE};
use warp::reply::Reply;
use warp::Filter;

#[derive(Parser, Debug)]
#[command(name = "lift", version, about, long_about = None)]
struct Args {
    /// Transfer the target as a downloadable file instead of rendering as HTML/text
    #[arg(short = 'f')]
    file_mode: bool,

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

    #[arg(
        value_name = "REMOTE",
        short = 'r',
        long = "remote",
        default_value = "bore.pub"
    )]
    bore_remote: String,
}

enum Payload {
    Text(String),
    File { bytes: Vec<u8>, filename: String },
}

fn random_path() -> String {
    rand::distr::Alphanumeric.sample_string(&mut rand::rng(), 8)
}

async fn serve(payload: Payload, max_count: u32, max_seconds: u64, bore_remote: String) {
    let r = random_path();

    let listener = tokio::net::TcpListener::bind("0.0.0.0:0").await.unwrap();
    let local_port = listener.local_addr().unwrap().port();

    let client = Client::new("localhost", local_port, &bore_remote, 0, None)
        .await
        .expect("Failed to create bore");
    let remote_port = client.remote_port();

    tokio::spawn(async move {
        if let Err(e) = client.listen().await {
            eprintln!("Bore tunnel error: {:?}", e);
        }
    });

    let (tx, mut rx) = mpsc::channel::<()>(1);
    let tx_mx = Arc::new(Mutex::new(Some(tx)));
    let counter = Arc::new(AtomicU32::new(max_count));

    let r_filter = r.clone();
    let payload = Arc::new(payload);

    let route = warp::path::param::<String>()
        .and(warp::path::end())
        .and_then(move |seg: String| {
            let payload = payload.clone();
            let tx_mx = tx_mx.clone();
            let r_filter = r_filter.clone();
            let counter = counter.clone();

            async move {
                if seg != r_filter {
                    return Err(warp::reject::not_found());
                }

                let current = counter.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |val| {
                    if val > 0 { Some(val - 1) } else { None }
                });

                match current {
                    Err(_) => return Err(warp::reject::not_found()),
                    Ok(prev) => {
                        if prev == 1 {
                            let tx_mx = tx_mx.clone();
                            tokio::spawn(async move {
                                if let Ok(mut guard) = tx_mx.lock() {
                                    if let Some(sender) = guard.take() {
                                        let _ = sender.send(());
                                    }
                                }
                            });
                        }
                    }
                }

                let response = match &*payload {
                    Payload::File { bytes, filename } => warp::http::Response::builder()
                        .header(CONTENT_TYPE, "application/octet-stream")
                        .header(
                            CONTENT_DISPOSITION,
                            format!("attachment; filename=\"{}\"", filename),
                        )
                        .body(bytes.clone())
                        .unwrap()
                        .into_response(),
                    Payload::Text(html_string) => {
                        warp::reply::html(html_string.clone()).into_response()
                    }
                };
                Ok::<_, warp::Rejection>(response)
            }
        });

    println!(
        "Data available at http://{}:{}/{}",
        bore_remote, remote_port, r
    );

    if max_seconds > 0 {
        println!(
            "Expires after {} download(s) or {} second(s).",
            max_count, max_seconds
        );
    } else {
        println!("Expires after {} download(s).", max_count);
    }

    let run = async move {
        warp::serve(route)
            .incoming(listener)
            .graceful(async move {
                let _ = rx.recv().await;
                println!("Maximum request count reached, closing remote.")
            })
            .run()
            .await;
    };

    if max_seconds > 0 {
        let res = tokio::time::timeout(Duration::from_secs(max_seconds), run).await;
        if res.is_err() {
            println!(
                "Timeout of {} seconds reached, closing remote.",
                max_seconds
            );
        }
    } else {
        run.await;
    }
}

fn from_stdin() -> String {
    let mut data = String::new();
    std::io::stdin()
        .read_to_string(&mut data)
        .expect("Failed to read from stdin");
    data
}

fn from_file_bytes(path: &str) -> Vec<u8> {
    std::fs::read(path).expect("Failed to read file bytes")
}

fn extract_filename(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    let payload = match args.filename {
        None => {
            if args.file_mode {
                eprintln!("Error: Missing filename after -f");
                process::exit(1);
            }
            Payload::Text(from_stdin())
        }
        Some(path) => {
            if args.file_mode {
                let bytes = from_file_bytes(&path);
                let filename = extract_filename(&path);
                Payload::File { bytes, filename }
            } else {
                let text = std::fs::read_to_string(&path).expect("Failed to read text file");
                Payload::Text(text)
            }
        }
    };

    serve(payload, args.max_count, args.max_time, args.bore_remote).await;
}
