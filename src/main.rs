use bore_cli::client::Client;
use rand::distr::SampleString;
use std::env;
use std::io::Read;
use std::process;
use std::sync::{Arc, Mutex};
use tokio::sync::oneshot;
use warp::Filter;
use warp::http::header::{CONTENT_DISPOSITION, CONTENT_TYPE};
use warp::reply::Reply;

enum Payload {
    Text(String),
    File { bytes: Vec<u8>, filename: String },
}

fn random_path() -> String {
    rand::distr::Alphanumeric.sample_string(&mut rand::rng(), 8)
}

async fn serve(payload: Payload) {
    let r = random_path();

    let listener = tokio::net::TcpListener::bind("0.0.0.0:0").await.unwrap();
    let local_port = listener.local_addr().unwrap().port();

    let client = Client::new("localhost", local_port, "bore.pub", 0, None).await.expect("Failed to create bore");
    let remote_port = client.remote_port();

    tokio::spawn(async move {
        if let Err(e) = client.listen().await {
            eprintln!("Bore tunnel error: {:?}", e);
        }
    });

    let (tx, rx) = oneshot::channel::<()>();
    let tx_mx = Arc::new(Mutex::new(Some(tx)));
    let r_filter = r.clone();

    let payload = Arc::new(payload);
    let route = warp::path::param::<String>()
        .and(warp::path::end())
        .and_then(move |seg: String| {
            let payload = payload.clone();
            let tx_mx = tx_mx.clone();
            let r_filter = r_filter.clone();
            async move {
                if seg != r_filter {
                    return Err(warp::reject::not_found());
                }

                if let Ok(mut guard) = tx_mx.lock() {
                    if let Some(sender) = guard.take() {
                        let _ = sender.send(());
                    }
                }

                let response = match &*payload {
                    Payload::File { bytes, filename } => {
                        warp::http::Response::builder()
                            .header(CONTENT_TYPE, "application/octet-stream")
                            .header(
                                CONTENT_DISPOSITION,
                                format!("attachment; filename=\"{}\"", filename),
                            )
                            .body(bytes.clone())
                            .unwrap().into_response()
                    }
                    Payload::Text(html_string) => {
                        warp::reply::html(html_string.clone()).into_response()
                    }
                };
                Ok::<_, warp::Rejection>(response)
            }
        });

    println!("Data available at http://bore.pub:{}/{}", remote_port, r);

    warp::serve(route)
        .incoming(listener)
        .graceful(async move {
            let _ = rx.await;
        })
        .run()
        .await;
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
    let args: Vec<String> = env::args().collect();

    let payload = match args.len() {
        1 => Payload::Text(from_stdin()),
        2 => {
            if args[1] == "-f" {
                eprintln!("Error: Missing filename after -f");
                process::exit(1);
            }
            if args[1] == "--version" {
                println!("lift v{}", env!("CARGO_PKG_VERSION"));
                process::exit(0);
            }
            if args[1] == "--help" || args[1] == "-h" {
                println!("Usage: lift [-f filename | filename]");
                println!("Subcommands:");
                println!("  --help: display this help message");
                println!("  --version: display the program version");
                println!("If no file path is specified, read from stdin.");
                println!("[-f]ile mode makes it so instead of presenting your data as text / html, \
                it is transferred as a downloadable file.");
                process::exit(0);
            }
            Payload::Text(std::fs::read_to_string(&args[1]).expect("Failed to read text file"))
        }
        3 => {
            if args[1] == "-f" {
                let bytes = from_file_bytes(&args[2]);
                let filename = extract_filename(&args[2]);
                Payload::File { bytes, filename }
            } else {
                eprintln!("Usage: lift [-f filename | filename]");
                process::exit(1);
            }
        }
        _ => {
            eprintln!("Usage: lift [-f filename | filename]");
            eprintln!("Try lift --help for more information.");
            process::exit(1);
        }
    };

    serve(payload).await;
}
