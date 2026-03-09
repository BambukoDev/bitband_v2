use core::error;

use embassy_net::{tcp::TcpSocket, Stack};
use embassy_time::{Duration, Timer};
use picoserve::routing::get_service;
use picoserve::{
    routing::get,
    routing::post,
    Config, Router,
    Server, Timeouts,
};
use defmt::{info, error};
use picoserve::response::{Directory, File, StatusCode};
use crate::services::ducky::DUCKY_CH;
use crate::ui::menu_core::MAX_NAME;
use alloc::string::String;

#[derive(serde::Deserialize)]
struct RunQuery {
    file: String,
}

fn make_router() -> Router<impl picoserve::routing::PathRouter> {
    Router::new()
        .route("/", get_service(File::html(include_str!("../webpage/index.html"))))
        .nest_service("/static", const {
            Directory {
                files: &[("index.css", File::css(include_str!("../webpage/style.css")))],
                ..Directory::DEFAULT
            }
        })
        .route("/run", post(handle_run_ducky))
}

async fn handle_run_ducky(
    picoserve::extract::Query(query): picoserve::extract::Query<RunQuery>
) -> impl picoserve::response::IntoResponse {
    let filename_raw = &query.file;
    
    let mut filename = heapless::String::<MAX_NAME>::new();
    
    if filename.push_str(filename_raw).is_err() {
        return (StatusCode::BAD_REQUEST, "Filename too long");
    }

    if DUCKY_CH.try_send(filename).is_err() {
        defmt::error!("Ducky channel full, rejecting request for {}", filename_raw.as_str());
        return (StatusCode::SERVICE_UNAVAILABLE, "System Busy");
    }

    defmt::info!("Web command: Queued {}", filename_raw.as_str());
    (StatusCode::OK, "OK")
}

#[embassy_executor::task]
pub async fn web_server_task(stack: &'static Stack<'static>) {
    // Buffers for the TCP stack
    let mut rx_buffer = [0; 1024];
    let mut tx_buffer = [0; 1024];
    
    // Buffer for the HTTP server's internal operations
    let mut http_buffer = [0; 2048];

    let config = Config::new(picoserve::Timeouts::default());
    let router = make_router();

    loop {
        if !stack.is_link_up() {
            Timer::after(Duration::from_millis(500)).await;
            continue;
        }

        let mut socket = TcpSocket::new(*stack, &mut rx_buffer, &mut tx_buffer);
        Timer::after(Duration::from_millis(10)).await;
        
        if let Err(e) = socket.accept(80).await {
            error!("Socket failed: {:?}", e);
            Timer::after(Duration::from_millis(100)).await;
            continue;
        }

        let server = Server::new(&router, &config, &mut http_buffer);

        match server.serve(socket).await {
            Ok(_) => defmt::info!("HTTP session closed"),
            Err(e) => defmt::error!("Picoserve error: {:?}", e),
        }
    }
}
