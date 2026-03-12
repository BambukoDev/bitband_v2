use core::error;
use core::sync::atomic::Ordering;

use alloc::vec::Vec;
use embassy_net::{tcp::TcpSocket, Stack};
use embassy_time::{Duration, Timer};
use picoserve::extract::{Form, Json, Query};
use picoserve::routing::get_service;
use picoserve::{
    routing::get,
    routing::post,
    Config, Router,
    Server, Timeouts,
};
use defmt::{info, error, warn};
use picoserve::response::{Directory, File, StatusCode};
use crate::services::nvs;
use crate::ui::menu_core::MAX_NAME;
use alloc::string::{String, ToString};
use crate::services::wifi::{WifiMode, WIFI_MODE_SIGNAL};
use crate::services::ducky::{DIRECT_EXEC_CH, DUCKY_CH, DUCKY_STATE, PAUSE_BUTTON_CH, READ_FILE_CH, READ_FILE_CONTENTS};

#[derive(serde::Deserialize)]
struct RunQuery {
    file: String,
}

#[derive(serde::Deserialize)]
struct ReadQuery {
    file: String,
}

#[derive(serde::Deserialize)]
struct WifiCredentials {
    ssid: String,
    pass: String,
}

#[derive(serde::Deserialize)]
struct DuckyForm {
    script: String,
}

#[derive(serde::Serialize)]
struct FileListResponse {
    files: Vec<String>,
}

#[derive(serde::Serialize)]
struct FileContentResponse {
    contents: String,
}

#[derive(serde::Serialize)]
struct StatusResponse {
    state: &'static str,
}

fn make_router() -> Router<impl picoserve::routing::PathRouter> {
    Router::new()
        .route("/", get_service(File::html(include_str!("../webpage/index.html"))))
        .route("/run", post(handle_run_ducky))
        .route("/run_raw", post(handle_run_raw))
        .route("/resume", post(handle_resume))
        .route("/status", get(handle_get_status))
        .route("/configure_wifi", post(handle_wifi_config)) 
        .route("/list_files", get(handle_list_files))
        .route("/get_file", post(handle_get_file))
}

// IMPL for get_file
async fn handle_get_file(
    Json(file): Json<ReadQuery>
) -> impl picoserve::response::IntoResponse {
    if file.file.is_empty() {
        // return (StatusCode::BAD_REQUEST, "Empty filename");
        return picoserve::response::Json(FileContentResponse { contents: "".to_string() });
    }
    if READ_FILE_CH.try_send(file.file).is_err() {
        // return (StatusCode::SERVICE_UNAVAILABLE, "System Busy");
        return picoserve::response::Json(FileContentResponse { contents: "".to_string() });
    }
    
    Timer::after_millis(100).await;
    let contents = READ_FILE_CONTENTS.receive().await;

    picoserve::response::Json(FileContentResponse { contents })
}

async fn handle_get_status() -> impl picoserve::response::IntoResponse {
    let state = match DUCKY_STATE.load(Ordering::Relaxed) {
        1 => "running",
        2 => "paused",
        _ => "idle",
    };
    picoserve::response::Json(StatusResponse { state })
}

async fn handle_run_raw(
    Form(payload): Form<DuckyForm>
) -> impl picoserve::response::IntoResponse {
    if payload.script.is_empty() {
        return (StatusCode::BAD_REQUEST, "Empty script");
    }

    // Send the decoded string to the Ducky channel
    if DIRECT_EXEC_CH.try_send(payload.script).is_err() {
        return (StatusCode::SERVICE_UNAVAILABLE, "System Busy");
    }

    (StatusCode::OK, "Executing Form Script...")
}

async fn handle_resume() -> impl picoserve::response::IntoResponse {
    let _ = PAUSE_BUTTON_CH.try_send(());
    (StatusCode::OK, "Resumed")
}

async fn handle_wifi_config(
    picoserve::extract::Json(creds): picoserve::extract::Json<WifiCredentials>
) -> impl picoserve::response::IntoResponse {
    nvs::save_wifi_credentials(creds.ssid.as_str(), creds.pass.as_str());
    WIFI_MODE_SIGNAL.signal(WifiMode::Sta(creds.ssid, creds.pass));

    (StatusCode::OK, "Switching to Station Mode...")
}

async fn handle_list_files() -> impl picoserve::response::IntoResponse {
    unsafe {
        crate::services::sd_monitor::FILES.lock_mut(|f| {
            let mut files = Vec::new();
            for entry in f.iter() {
                files.push(entry.name.to_string());
            }
            picoserve::response::Json(FileListResponse { files })
        })
    }
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

    defmt::println!("Web command: Queued {}", filename_raw.as_str());
    (StatusCode::OK, "OK")
}

#[embassy_executor::task]
pub async fn web_server_task(
    ap_stack: &'static Stack<'static>,
    sta_stack: &'static Stack<'static>
) {
    // Buffers for the TCP stack
    let mut rx_buffer = [0; 4096];
    let mut tx_buffer = [0; 1024];
    
    // Buffer for the HTTP server's internal operations
    let mut http_buffer = [0; 8192];

    let config = Config::new(picoserve::Timeouts::default());
    let router = make_router();

    loop {
        let stack = if ap_stack.is_link_up() { ap_stack } 
                    else if sta_stack.is_link_up() { sta_stack } 
                    else { Timer::after_millis(500).await; continue; };

        let mut socket = TcpSocket::new(*stack, &mut rx_buffer, &mut tx_buffer);
        socket.set_timeout(Some(Duration::from_secs(10)));

        if let Err(_e) = socket.accept(80).await { continue; }

        let server = Server::new(&router, &config, &mut http_buffer);
        
        match server.serve(socket).await {
            Ok(_) => {},
            Err(e) => error!("Picoserve error: {:?}", e),
        }
        Timer::after_millis(50).await;
    }
}
