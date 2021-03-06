extern crate clap;
extern crate flate2;
extern crate hyper;
#[macro_use]
extern crate lazy_static;
extern crate r2d2;
extern crate r2d2_sqlite;
extern crate regex;
extern crate serde;
extern crate serde_json;

use hyper::service::{make_service_fn, service_fn};
use hyper::Server;
use std::process;

mod config;
mod errors;
mod service;
mod tiles;
mod utils;

#[tokio::main]
async fn main() {
    pretty_env_logger::init();

    let args = match config::parse(config::get_app().get_matches()) {
        Ok(args) => args,
        Err(err) => {
            println!("{}", err);
            process::exit(1)
        }
    };

    println!("Serving tiles from {}", args.directory.display());

    let tilesets = tiles::discover_tilesets(String::new(), args.directory);

    let addr = ([0, 0, 0, 0], args.port).into();

    let allowed_hosts = args.allowed_hosts;
    let headers = args.headers;

    let disable_preview = args.disable_preview;

    let make_service = make_service_fn(move |_conn| {
        let tilesets = tilesets.clone();
        let allowed_hosts = allowed_hosts.clone();
        let headers = headers.clone();
        async move {
            Ok::<_, hyper::Error>(service_fn(move |req| {
                service::get_service(
                    req,
                    tilesets.clone(),
                    allowed_hosts.clone(),
                    headers.clone(),
                    disable_preview,
                )
            }))
        }
    });

    let server = match Server::try_bind(&addr) {
        Ok(builder) => builder.serve(make_service),
        Err(err) => {
            println!("{}", err);
            process::exit(1);
        }
    };

    println!("Listening on http://{}", addr);

    if let Err(e) = server.await {
        eprintln!("server error: {}", e);
    }
}
